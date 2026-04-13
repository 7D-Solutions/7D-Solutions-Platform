//! Production invariant checks.
//!
//! Invariants:
//! 1. completed_wo_output_cap: For closed work orders, completed_quantity
//!    must be <= planned_quantity * 1.1 (10% overrun tolerance).
//!    An overrun beyond 10% indicates an operator entry error or a guard bypass.
//! 2. closed_wo_has_actual_end: Every work order with status='closed' must
//!    have actual_end IS NOT NULL. A closed WO without an end timestamp
//!    indicates an incomplete close workflow.
//!
//! Note on component issue invariant:
//! The bead specification includes "sum(issued_qty) >= bom_required_qty for
//! 'closed' WOs". This invariant requires cross-database queries joining the
//! production database (work_orders, bom_revision_id) with the inventory
//! database (inventory_ledger quantities issued to work_order references) and
//! the BOM database (bom_lines required quantities). Cross-DB joins are not
//! supported in a single reconciliation pass without a read-only data warehouse.
//! This invariant is documented and deferred to a future cross-module recon bead.
//!
//! SQL forms (for manual verification):
//! ```sql
//! -- Invariant 1: completed_wo_output_cap
//! SELECT work_order_id, tenant_id, planned_quantity, completed_quantity,
//!        planned_quantity * 1.1 AS max_allowed
//! FROM work_orders
//! WHERE status = 'closed'
//!   AND completed_quantity > planned_quantity * 1.1;
//!
//! -- Invariant 2: closed_wo_has_actual_end
//! SELECT work_order_id, tenant_id, status, actual_end
//! FROM work_orders
//! WHERE status = 'closed'
//!   AND actual_end IS NULL;
//! ```

use anyhow::Result;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use super::Violation;

const MODULE: &str = "production";

/// Run all production invariant checks. Returns list of violations found.
pub async fn run_checks(pool: &PgPool) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();

    info!("Production: checking completed_wo_output_cap invariant");
    violations.extend(check_completed_wo_output_cap(pool).await?);

    info!("Production: checking closed_wo_has_actual_end invariant");
    violations.extend(check_closed_wo_has_actual_end(pool).await?);

    Ok(violations)
}

/// Invariant 1: Closed work order completed quantity does not exceed 110% of planned.
///
/// A 10% overrun tolerance is standard in manufacturing for scrap and trial pieces.
/// Beyond 10% indicates either a data entry error in reporting completion or
/// a guard bypass that allowed recording excessive output.
async fn check_completed_wo_output_cap(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String, i32, i32)> = sqlx::query_as(
        r#"
        SELECT work_order_id,
               tenant_id,
               planned_quantity,
               completed_quantity
        FROM work_orders
        WHERE status = 'closed'
          AND planned_quantity > 0
          AND completed_quantity > CAST(planned_quantity * 1.1 AS INTEGER)
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (wo_id, tenant_id, planned, completed) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "completed_wo_output_cap",
        rows.len() as i64,
        format!(
            "first violation: work_order_id={wo_id} tenant_id={tenant_id} planned={planned} completed={completed} max_allowed={}",
            (*planned as f64 * 1.1) as i64
        ),
    )])
}

/// Invariant 2: Closed work orders must have an actual_end timestamp.
///
/// When a work order is closed, the close handler sets actual_end = NOW().
/// A closed WO without actual_end indicates the close workflow was bypassed
/// or an incomplete state transition was persisted.
async fn check_closed_wo_has_actual_end(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        r#"
        SELECT work_order_id, tenant_id
        FROM work_orders
        WHERE status = 'closed'
          AND actual_end IS NULL
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (wo_id, tenant_id) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "closed_wo_has_actual_end",
        rows.len() as i64,
        format!("first violation: work_order_id={wo_id} tenant_id={tenant_id}"),
    )])
}

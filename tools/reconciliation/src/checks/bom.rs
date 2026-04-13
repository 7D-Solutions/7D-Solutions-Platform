//! BOM (Bill of Materials) invariant checks.
//!
//! Invariants:
//! 1. revision_status_valid: Every bom_revision has status in the allowed set
//!    ('draft', 'effective', 'superseded'). A NULL or unknown status indicates
//!    a migration gap or direct DB write bypassing the application.
//! 2. effective_bom_no_zero_qty: bom_lines belonging to an 'effective' revision
//!    must have quantity > 0. A zero-quantity component on a released BOM would
//!    result in production planning calculating zero material requirements.
//!
//! SQL forms (for manual verification):
//! ```sql
//! -- Invariant 1: revision_status_valid
//! SELECT id, bom_id, tenant_id, revision_label, status
//! FROM bom_revisions
//! WHERE status IS NULL
//!    OR status NOT IN ('draft', 'effective', 'superseded');
//!
//! -- Invariant 2: effective_bom_no_zero_qty
//! SELECT bl.id, bl.revision_id, bl.tenant_id, bl.component_item_id, bl.quantity
//! FROM bom_lines bl
//! JOIN bom_revisions br ON br.id = bl.revision_id
//! WHERE br.status = 'effective'
//!   AND bl.quantity = 0;
//! ```

use anyhow::Result;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use super::Violation;

const MODULE: &str = "bom";

/// Run all BOM invariant checks. Returns list of violations found.
pub async fn run_checks(pool: &PgPool) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();

    info!("BOM: checking revision_status_valid invariant");
    violations.extend(check_revision_status_valid(pool).await?);

    info!("BOM: checking effective_bom_no_zero_qty invariant");
    violations.extend(check_effective_bom_no_zero_qty(pool).await?);

    Ok(violations)
}

/// Invariant 1: All BOM revisions have a valid status.
///
/// The allowed status values are enforced by a database CHECK constraint,
/// but that constraint was added via migration and may not cover rows
/// inserted before the constraint existed. This check catches any rows
/// with NULL or unexpected status values that slipped through.
async fn check_revision_status_valid(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, tenant_id, status
        FROM bom_revisions
        WHERE status IS NULL
           OR status NOT IN ('draft', 'effective', 'superseded')
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (revision_id, tenant_id, status) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "revision_status_valid",
        rows.len() as i64,
        format!(
            "first violation: revision_id={revision_id} tenant_id={tenant_id} status={status:?}"
        ),
    )])
}

/// Invariant 2: Effective (released) BOM lines must have quantity > 0.
///
/// A bom_line.quantity of 0 on an effective BOM would cause MRP and production
/// planning to calculate zero material requirements for that component — silently
/// producing incorrect production orders. The CHECK constraint (quantity > 0)
/// should prevent this at insert time; this check detects any that slipped through.
async fn check_effective_bom_no_zero_qty(pool: &PgPool) -> Result<Vec<Violation>> {
    // The bom_lines.quantity column has a CHECK (quantity > 0) constraint,
    // but we verify at the data level to catch pre-constraint rows.
    let rows: Vec<(Uuid, String, Uuid)> = sqlx::query_as(
        r#"
        SELECT bl.id, bl.tenant_id, bl.revision_id
        FROM bom_lines bl
        JOIN bom_revisions br ON br.id = bl.revision_id
        WHERE br.status = 'effective'
          AND bl.quantity <= 0
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (line_id, tenant_id, revision_id) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "effective_bom_no_zero_qty",
        rows.len() as i64,
        format!(
            "first violation: line_id={line_id} tenant_id={tenant_id} revision_id={revision_id}"
        ),
    )])
}

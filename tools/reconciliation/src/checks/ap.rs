//! AP (Accounts Payable) invariant checks.
//!
//! Invariants:
//! 1. bill_line_total: vendor_bills.total_minor = SUM(bill_lines.line_total_minor)
//!    + COALESCE(tax_minor, 0) for non-voided bills.
//! 2. payment_allocation_cap: SUM(ap_allocations.amount_minor per bill)
//!    <= vendor_bills.total_minor for non-voided bills.
//!
//! SQL forms (for manual verification):
//! ```sql
//! -- Invariant 1: bill_line_total
//! SELECT b.bill_id, b.tenant_id,
//!        b.total_minor AS stored,
//!        COALESCE(SUM(l.line_total_minor), 0) + COALESCE(b.tax_minor, 0) AS computed
//! FROM vendor_bills b
//! LEFT JOIN bill_lines l ON l.bill_id = b.bill_id
//! WHERE b.status NOT IN ('voided')
//! GROUP BY b.bill_id, b.tenant_id, b.total_minor, b.tax_minor
//! HAVING b.total_minor <> COALESCE(SUM(l.line_total_minor), 0) + COALESCE(b.tax_minor, 0);
//!
//! -- Invariant 2: payment_allocation_cap
//! SELECT b.bill_id, b.tenant_id, b.total_minor,
//!        COALESCE(SUM(a.amount_minor), 0) AS total_allocated
//! FROM vendor_bills b
//! LEFT JOIN ap_allocations a ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
//! WHERE b.status NOT IN ('voided')
//! GROUP BY b.bill_id, b.tenant_id, b.total_minor
//! HAVING COALESCE(SUM(a.amount_minor), 0) > b.total_minor;
//! ```

use anyhow::Result;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use super::Violation;

const MODULE: &str = "ap";

/// Run all AP invariant checks. Returns list of violations found.
pub async fn run_checks(pool: &PgPool) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();

    info!("AP: checking bill_line_total invariant");
    violations.extend(check_bill_line_total(pool).await?);

    info!("AP: checking payment_allocation_cap invariant");
    violations.extend(check_payment_allocation_cap(pool).await?);

    Ok(violations)
}

/// Invariant 1: Bill total matches sum of line totals plus tax.
///
/// vendor_bills.total_minor must equal SUM(bill_lines.line_total_minor) +
/// COALESCE(vendor_bills.tax_minor, 0) for all non-voided bills.
/// A mismatch indicates a line was updated without recalculating the bill header.
async fn check_bill_line_total(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT b.bill_id,
               b.tenant_id,
               b.total_minor AS stored_total,
               (COALESCE(line_sum.total, 0) + COALESCE(b.tax_minor, 0))::BIGINT AS computed_total
        FROM vendor_bills b
        LEFT JOIN (
            SELECT bill_id, SUM(line_total_minor) AS total
            FROM bill_lines
            GROUP BY bill_id
        ) line_sum ON line_sum.bill_id = b.bill_id
        WHERE b.status NOT IN ('voided')
          AND b.total_minor <> COALESCE(line_sum.total, 0) + COALESCE(b.tax_minor, 0)
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (bill_id, tenant_id, stored, computed) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "bill_line_total",
        rows.len() as i64,
        format!(
            "first violation: bill_id={bill_id} tenant_id={tenant_id} stored={stored} computed={computed}"
        ),
    )])
}

/// Invariant 2: Total allocated payments do not exceed bill amount.
///
/// The sum of all ap_allocations for a given bill must not exceed
/// vendor_bills.total_minor. Over-allocation indicates the payment allocation
/// logic applied more than the bill value.
async fn check_payment_allocation_cap(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT b.bill_id,
               b.tenant_id,
               b.total_minor,
               COALESCE(SUM(a.amount_minor), 0)::BIGINT AS total_allocated
        FROM vendor_bills b
        LEFT JOIN ap_allocations a ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
        WHERE b.status NOT IN ('voided')
        GROUP BY b.bill_id, b.tenant_id, b.total_minor
        HAVING COALESCE(SUM(a.amount_minor), 0) > b.total_minor
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (bill_id, tenant_id, cap, allocated) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "payment_allocation_cap",
        rows.len() as i64,
        format!(
            "first violation: bill_id={bill_id} tenant_id={tenant_id} cap={cap} allocated={allocated}"
        ),
    )])
}

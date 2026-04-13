//! AR (Accounts Receivable) invariant checks.
//!
//! Invariants:
//! 1. invoice_line_total: ar_invoices.amount_cents = SUM(line_items.amount_cents)
//!    + COALESCE(SUM(tax_calculations.tax_amount_cents), 0) for all non-void invoices.
//! 2. payment_allocation_cap: SUM(ar_payment_allocations.amount_cents per invoice)
//!    <= ar_invoices.amount_cents for all non-void invoices.
//!
//! SQL forms (for manual verification):
//! ```sql
//! -- Invariant 1: invoice_line_total
//! SELECT i.id, i.app_id, i.amount_cents AS stored,
//!        COALESCE(SUM(l.amount_cents), 0) + COALESCE(SUM(t.tax_amount_cents), 0) AS computed
//! FROM ar_invoices i
//! LEFT JOIN ar_invoice_line_items l ON l.invoice_id = i.id AND l.app_id = i.app_id
//! LEFT JOIN ar_tax_calculations t  ON t.invoice_id = i.id AND t.app_id = i.app_id
//! WHERE i.status NOT IN ('void', 'voided')
//! GROUP BY i.id, i.app_id, i.amount_cents
//! HAVING i.amount_cents <> COALESCE(SUM(l.amount_cents), 0) + COALESCE(SUM(t.tax_amount_cents), 0);
//!
//! -- Invariant 2: payment_allocation_cap
//! SELECT i.id, i.app_id, i.amount_cents,
//!        COALESCE(SUM(a.amount_cents), 0) AS total_allocated
//! FROM ar_invoices i
//! LEFT JOIN ar_payment_allocations a ON a.invoice_id = i.id AND a.app_id = i.app_id
//! WHERE i.status NOT IN ('void', 'voided')
//! GROUP BY i.id, i.app_id, i.amount_cents
//! HAVING COALESCE(SUM(a.amount_cents), 0) > i.amount_cents;
//! ```

use anyhow::Result;
use sqlx::PgPool;
use tracing::info;

use super::Violation;

const MODULE: &str = "ar";

/// Run all AR invariant checks. Returns list of violations found.
pub async fn run_checks(pool: &PgPool) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();

    info!("AR: checking invoice_line_total invariant");
    violations.extend(check_invoice_line_total(pool).await?);

    info!("AR: checking payment_allocation_cap invariant");
    violations.extend(check_payment_allocation_cap(pool).await?);

    Ok(violations)
}

/// Invariant 1: Invoice amount matches sum of line items + tax.
///
/// Checks that ar_invoices.amount_cents equals the sum of all associated
/// ar_invoice_line_items.amount_cents plus ar_tax_calculations.tax_amount_cents
/// for non-voided invoices. A mismatch indicates a data corruption or missed
/// update when lines were added/removed after the invoice total was set.
///
/// Note: invoices with no line items and no tax are expected to have amount_cents
/// matching 0. If amount_cents = 0 with no lines, no violation is raised.
async fn check_invoice_line_total(pool: &PgPool) -> Result<Vec<Violation>> {
    // Returns rows where the stored total differs from the sum of components.
    // NULL-safe: COALESCE ensures sums default to 0 when no line items or tax exist.
    let rows: Vec<(i32, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT i.id,
               i.app_id,
               i.amount_cents::BIGINT AS stored_total,
               (COALESCE(line_sum.total, 0) + COALESCE(tax_sum.total, 0))::BIGINT AS computed_total
        FROM ar_invoices i
        LEFT JOIN (
            SELECT app_id, invoice_id, SUM(amount_cents) AS total
            FROM ar_invoice_line_items
            GROUP BY app_id, invoice_id
        ) line_sum ON line_sum.invoice_id = i.id AND line_sum.app_id = i.app_id
        LEFT JOIN (
            SELECT app_id, invoice_id, SUM(tax_amount_cents) AS total
            FROM ar_tax_calculations
            GROUP BY app_id, invoice_id
        ) tax_sum ON tax_sum.invoice_id = i.id AND tax_sum.app_id = i.app_id
        WHERE i.status NOT IN ('void', 'voided')
          AND i.amount_cents <> COALESCE(line_sum.total, 0) + COALESCE(tax_sum.total, 0)
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (invoice_id, app_id, stored, computed) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "invoice_line_total",
        rows.len() as i64,
        format!(
            "first violation: invoice_id={invoice_id} app_id={app_id} stored={stored} computed={computed}"
        ),
    )])
}

/// Invariant 2: Total allocated payments do not exceed invoice amount.
///
/// Checks that the sum of all ar_payment_allocations for a given invoice
/// does not exceed the invoice's amount_cents. Overpayment beyond invoice
/// face value indicates allocation logic error or double-allocation.
async fn check_payment_allocation_cap(pool: &PgPool) -> Result<Vec<Violation>> {
    let rows: Vec<(i32, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT i.id,
               i.app_id,
               i.amount_cents::BIGINT,
               COALESCE(SUM(a.amount_cents), 0)::BIGINT AS total_allocated
        FROM ar_invoices i
        LEFT JOIN ar_payment_allocations a ON a.invoice_id = i.id AND a.app_id = i.app_id
        WHERE i.status NOT IN ('void', 'voided')
        GROUP BY i.id, i.app_id, i.amount_cents
        HAVING COALESCE(SUM(a.amount_cents), 0) > i.amount_cents
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let (invoice_id, app_id, cap, allocated) = &rows[0];
    Ok(vec![Violation::new(
        MODULE,
        "payment_allocation_cap",
        rows.len() as i64,
        format!(
            "first violation: invoice_id={invoice_id} app_id={app_id} cap={cap} allocated={allocated}"
        ),
    )])
}

//! Billing service — billing rates + billing runs.
//!
//! A billing run collects all unbilled, billable entries in a date range,
//! computes the total amount, records the run, and returns a result that
//! the caller can use to create an AR invoice.
//!
//! ## Idempotency
//! A billing run is keyed by (app_id, from_date, to_date, ar_customer_id).
//! Repeating the same run returns the existing result with `already_ran = true`.
//!
//! ## No double-billing invariant
//! Entries are included in at most one billing run. Once linked to a run via
//! `tk_billing_run_entries`, they are excluded from future runs.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use super::repo;

// ============================================================================
// Billing Rates
// ============================================================================

/// Create a new billing rate.
pub async fn create_billing_rate(
    pool: &PgPool,
    req: &CreateBillingRateRequest,
) -> Result<BillingRate, BillingError> {
    if req.app_id.trim().is_empty() {
        return Err(BillingError::Validation("app_id must not be empty".into()));
    }
    if req.name.trim().is_empty() {
        return Err(BillingError::Validation("name must not be empty".into()));
    }
    if req.rate_cents_per_hour <= 0 {
        return Err(BillingError::Validation(
            "rate_cents_per_hour must be positive".into(),
        ));
    }

    repo::insert_billing_rate(pool, &req.app_id, &req.name, req.rate_cents_per_hour).await
}

/// List active billing rates for an app.
pub async fn list_billing_rates(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<BillingRate>, BillingError> {
    repo::list_billing_rates(pool, app_id).await
}

// ============================================================================
// Billing Runs
// ============================================================================

/// Create a billing run for the given period and AR customer.
///
/// Collects all billable, unbilled entries (those with a billing_rate_id and
/// not yet in any billing run) for the period. Returns a `BillingRunResult`
/// including line items for AR invoice creation.
///
/// If a billing run already exists for this (app_id, from_date, to_date,
/// ar_customer_id), returns it with `already_ran = true` — no new AR invoice
/// should be created by the caller.
pub async fn create_billing_run(
    pool: &PgPool,
    req: &CreateBillingRunRequest,
) -> Result<BillingRunResult, BillingError> {
    if req.app_id.trim().is_empty() {
        return Err(BillingError::Validation("app_id must not be empty".into()));
    }
    if req.to_date < req.from_date {
        return Err(BillingError::Validation(
            "to_date must be >= from_date".into(),
        ));
    }

    let idempotency_key = format!(
        "{}:{}:{}:{}",
        req.app_id, req.from_date, req.to_date, req.ar_customer_id
    );

    // Idempotency check — return existing run if found
    if let Some(existing) = repo::find_run_by_idempotency_key(pool, &idempotency_key).await? {
        let rows = repo::load_run_entries(pool, existing.id, &req.app_id).await?;
        let line_items = rows
            .into_iter()
            .map(|(entry_id, amount_cents)| BillingLineItem {
                entry_id,
                minutes: 0,
                rate_cents_per_hour: 0,
                amount_cents,
                description: None,
            })
            .collect();
        return Ok(BillingRunResult {
            run: existing,
            line_items,
            already_ran: true,
        });
    }

    // Collect unbilled billable entries for the period
    let entries =
        repo::fetch_billable_entries(pool, &req.app_id, req.from_date, req.to_date).await?;

    if entries.is_empty() {
        return Err(BillingError::NoBillableEntries);
    }

    // Compute line items
    let line_items: Vec<BillingLineItem> = entries
        .iter()
        .map(|e| {
            let amount_cents = (e.minutes as i64 * e.rate_cents_per_hour as i64 + 59) / 60;
            BillingLineItem {
                entry_id: e.entry_id,
                minutes: e.minutes,
                rate_cents_per_hour: e.rate_cents_per_hour,
                amount_cents,
                description: e.description.clone(),
            }
        })
        .collect();

    let total_cents: i64 = line_items.iter().map(|l| l.amount_cents).sum();

    // Create billing run + link entries atomically
    let run_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    let run = repo::insert_billing_run(
        &mut *tx,
        run_id,
        &req.app_id,
        req.ar_customer_id,
        req.from_date,
        req.to_date,
        total_cents,
        &idempotency_key,
    )
    .await?;

    for item in &line_items {
        repo::insert_billing_run_entry(&mut *tx, run_id, item.entry_id, item.amount_cents).await?;
    }

    tx.commit().await?;

    Ok(BillingRunResult {
        run,
        line_items,
        already_ran: false,
    })
}

/// Update the billing run with the AR invoice ID once the invoice is created.
pub async fn set_invoice_id(
    pool: &PgPool,
    run_id: Uuid,
    ar_invoice_id: i32,
) -> Result<(), BillingError> {
    repo::set_invoice_id(pool, run_id, ar_invoice_id).await
}

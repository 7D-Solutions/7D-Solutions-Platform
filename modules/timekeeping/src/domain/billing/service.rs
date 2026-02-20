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

    let rate = sqlx::query_as::<_, BillingRate>(
        r#"
        INSERT INTO tk_billing_rates (app_id, name, rate_cents_per_hour)
        VALUES ($1, $2, $3)
        RETURNING id, app_id, name, rate_cents_per_hour, is_active, created_at
        "#,
    )
    .bind(&req.app_id)
    .bind(&req.name)
    .bind(req.rate_cents_per_hour)
    .fetch_one(pool)
    .await?;

    Ok(rate)
}

/// List active billing rates for an app.
pub async fn list_billing_rates(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<BillingRate>, BillingError> {
    let rates = sqlx::query_as::<_, BillingRate>(
        r#"
        SELECT id, app_id, name, rate_cents_per_hour, is_active, created_at
        FROM tk_billing_rates
        WHERE app_id = $1 AND is_active = TRUE
        ORDER BY name
        "#,
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(rates)
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
    if let Some(existing) = find_existing_run(pool, &idempotency_key).await? {
        let line_items = load_run_entries(pool, existing.id).await?;
        return Ok(BillingRunResult {
            run: existing,
            line_items,
            already_ran: true,
        });
    }

    // Collect unbilled billable entries for the period
    let entries = fetch_billable_entries(pool, &req.app_id, req.from_date, req.to_date).await?;

    if entries.is_empty() {
        return Err(BillingError::NoBillableEntries);
    }

    // Compute line items
    let line_items: Vec<BillingLineItem> = entries
        .iter()
        .map(|e| {
            let amount_cents =
                (e.minutes as i64 * e.rate_cents_per_hour as i64 + 59) / 60;
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

    let run = sqlx::query_as::<_, BillingRun>(
        r#"
        INSERT INTO tk_billing_runs
            (id, app_id, ar_customer_id, from_date, to_date,
             amount_cents, idempotency_key, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'completed')
        RETURNING id, app_id, ar_customer_id, from_date, to_date,
                  amount_cents, ar_invoice_id, idempotency_key, status, created_at
        "#,
    )
    .bind(run_id)
    .bind(&req.app_id)
    .bind(req.ar_customer_id)
    .bind(req.from_date)
    .bind(req.to_date)
    .bind(total_cents)
    .bind(&idempotency_key)
    .fetch_one(&mut *tx)
    .await?;

    for item in &line_items {
        sqlx::query(
            r#"
            INSERT INTO tk_billing_run_entries (billing_run_id, entry_id, amount_cents)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(run_id)
        .bind(item.entry_id)
        .bind(item.amount_cents)
        .execute(&mut *tx)
        .await?;
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
    sqlx::query(
        "UPDATE tk_billing_runs SET ar_invoice_id = $1 WHERE id = $2",
    )
    .bind(ar_invoice_id)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Internal helpers
// ============================================================================

async fn find_existing_run(
    pool: &PgPool,
    idempotency_key: &str,
) -> Result<Option<BillingRun>, BillingError> {
    let run = sqlx::query_as::<_, BillingRun>(
        r#"
        SELECT id, app_id, ar_customer_id, from_date, to_date,
               amount_cents, ar_invoice_id, idempotency_key, status, created_at
        FROM tk_billing_runs
        WHERE idempotency_key = $1
        LIMIT 1
        "#,
    )
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await?;
    Ok(run)
}

/// Fetch entries that are billable, have a billing_rate_id, are current,
/// not void, and have not yet appeared in any billing run.
async fn fetch_billable_entries(
    pool: &PgPool,
    app_id: &str,
    from_date: chrono::NaiveDate,
    to_date: chrono::NaiveDate,
) -> Result<Vec<BillableEntryRow>, BillingError> {
    let rows = sqlx::query_as::<_, BillableEntryRow>(
        r#"
        SELECT
            e.entry_id,
            e.minutes,
            r.rate_cents_per_hour,
            e.description
        FROM tk_timesheet_entries e
        JOIN tk_billing_rates r ON r.id = e.billing_rate_id
        WHERE e.app_id = $1
          AND e.work_date >= $2
          AND e.work_date <= $3
          AND e.billable = TRUE
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
          AND e.billing_rate_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1 FROM tk_billing_run_entries bre
              WHERE bre.entry_id = e.entry_id
          )
        ORDER BY e.work_date, e.entry_id
        "#,
    )
    .bind(app_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Load previously-recorded line items for a billing run (for idempotent replay).
async fn load_run_entries(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<BillingLineItem>, BillingError> {
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT entry_id, amount_cents FROM tk_billing_run_entries WHERE billing_run_id = $1",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(entry_id, amount_cents)| BillingLineItem {
            entry_id,
            minutes: 0,             // not stored; not needed for idempotent replay
            rate_cents_per_hour: 0, // not stored; not needed for idempotent replay
            amount_cents,
            description: None,
        })
        .collect())
}

//! Billing repository — SQL layer for tk_billing_rates, tk_billing_runs, tk_billing_run_entries.

use chrono::NaiveDate;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::models::*;

// ============================================================================
// Billing Rates
// ============================================================================

pub async fn insert_billing_rate(
    pool: &PgPool,
    app_id: &str,
    name: &str,
    rate_cents_per_hour: i32,
) -> Result<BillingRate, BillingError> {
    Ok(sqlx::query_as::<_, BillingRate>(
        r#"
        INSERT INTO tk_billing_rates (app_id, name, rate_cents_per_hour)
        VALUES ($1, $2, $3)
        RETURNING id, app_id, name, rate_cents_per_hour, is_active, created_at
        "#,
    )
    .bind(app_id)
    .bind(name)
    .bind(rate_cents_per_hour)
    .fetch_one(pool)
    .await?)
}

pub async fn list_billing_rates(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<BillingRate>, BillingError> {
    Ok(sqlx::query_as::<_, BillingRate>(
        r#"
        SELECT id, app_id, name, rate_cents_per_hour, is_active, created_at
        FROM tk_billing_rates
        WHERE app_id = $1 AND is_active = TRUE
        ORDER BY name
        "#,
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

// ============================================================================
// Billing Runs
// ============================================================================

pub async fn find_run_by_idempotency_key(
    pool: &PgPool,
    idempotency_key: &str,
) -> Result<Option<BillingRun>, BillingError> {
    Ok(sqlx::query_as::<_, BillingRun>(
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
    .await?)
}

pub(in crate::domain::billing) async fn fetch_billable_entries(
    pool: &PgPool,
    app_id: &str,
    from_date: NaiveDate,
    to_date: NaiveDate,
) -> Result<Vec<BillableEntryRow>, BillingError> {
    Ok(sqlx::query_as::<_, BillableEntryRow>(
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
    .await?)
}

pub async fn load_run_entries(
    pool: &PgPool,
    run_id: Uuid,
    app_id: &str,
) -> Result<Vec<(Uuid, i64)>, BillingError> {
    Ok(sqlx::query_as(
        "SELECT entry_id, amount_cents FROM tk_billing_run_entries \
         WHERE billing_run_id = $1 \
           AND billing_run_id IN (SELECT id FROM tk_billing_runs WHERE app_id = $2)",
    )
    .bind(run_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

pub async fn insert_billing_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    app_id: &str,
    ar_customer_id: i32,
    from_date: NaiveDate,
    to_date: NaiveDate,
    amount_cents: i64,
    idempotency_key: &str,
) -> Result<BillingRun, BillingError> {
    Ok(sqlx::query_as::<_, BillingRun>(
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
    .bind(app_id)
    .bind(ar_customer_id)
    .bind(from_date)
    .bind(to_date)
    .bind(amount_cents)
    .bind(idempotency_key)
    .fetch_one(conn)
    .await?)
}

pub async fn insert_billing_run_entry(
    conn: &mut PgConnection,
    billing_run_id: Uuid,
    entry_id: Uuid,
    amount_cents: i64,
) -> Result<(), BillingError> {
    sqlx::query(
        r#"
        INSERT INTO tk_billing_run_entries (billing_run_id, entry_id, amount_cents)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(billing_run_id)
    .bind(entry_id)
    .bind(amount_cents)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn set_invoice_id(
    pool: &PgPool,
    run_id: Uuid,
    ar_invoice_id: i32,
) -> Result<(), BillingError> {
    sqlx::query("UPDATE tk_billing_runs SET ar_invoice_id = $1 WHERE id = $2")
        .bind(ar_invoice_id)
        .bind(run_id)
        .execute(pool)
        .await?;
    Ok(())
}

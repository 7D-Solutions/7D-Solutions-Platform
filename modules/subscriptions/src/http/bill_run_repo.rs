//! Bill run repository — all SQL operations for the bill run domain.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

// ============================================================================
// Row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub struct ExistingBillRun {
    #[allow(dead_code)]
    pub id: Uuid,
    pub subscriptions_processed: i32,
    pub invoices_created: i32,
    pub failures: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
pub struct SubscriptionDue {
    pub id: Uuid,
    #[allow(dead_code)]
    pub tenant_id: String,
    pub ar_customer_id: String,
    pub price_minor: i64,
    #[allow(dead_code)]
    pub currency: String,
    #[allow(dead_code)]
    pub next_bill_date: NaiveDate,
    pub schedule: String,
}

// ============================================================================
// Queries
// ============================================================================

/// Check if a bill run has already been executed (idempotency).
pub async fn fetch_existing_bill_run<'e>(
    executor: impl PgExecutor<'e>,
    bill_run_id: &str,
    tenant_id: &str,
) -> Result<Option<ExistingBillRun>, sqlx::Error> {
    sqlx::query_as::<_, ExistingBillRun>(
        "SELECT id, subscriptions_processed, invoices_created, failures, created_at
         FROM bill_runs
         WHERE bill_run_id = $1 AND tenant_id = $2",
    )
    .bind(bill_run_id)
    .bind(tenant_id)
    .fetch_optional(executor)
    .await
}

/// Insert a new bill run record with status 'running'.
pub async fn insert_bill_run<'e>(
    executor: impl PgExecutor<'e>,
    bill_run_id: &str,
    tenant_id: &str,
    execution_date: NaiveDate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bill_runs (bill_run_id, tenant_id, execution_date, status)
         VALUES ($1, $2, $3, 'running')",
    )
    .bind(bill_run_id)
    .bind(tenant_id)
    .bind(execution_date)
    .execute(executor)
    .await?;
    Ok(())
}

/// Fetch active subscriptions due for billing on or before execution_date.
pub async fn fetch_subscriptions_due<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    execution_date: NaiveDate,
) -> Result<Vec<SubscriptionDue>, sqlx::Error> {
    sqlx::query_as::<_, SubscriptionDue>(
        "SELECT id, tenant_id, ar_customer_id, price_minor, currency, next_bill_date, schedule
         FROM subscriptions
         WHERE status = 'active'
           AND tenant_id = $1
           AND next_bill_date <= $2",
    )
    .bind(tenant_id)
    .bind(execution_date)
    .fetch_all(executor)
    .await
}

/// Update a subscription's next_bill_date after successful billing.
pub async fn update_subscription_next_bill_date<'e>(
    executor: impl PgExecutor<'e>,
    subscription_id: Uuid,
    next_bill_date: NaiveDate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE subscriptions
         SET next_bill_date = $1, updated_at = NOW()
         WHERE id = $2",
    )
    .bind(next_bill_date)
    .bind(subscription_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Mark a bill run as completed with final counts.
pub async fn complete_bill_run<'e>(
    executor: impl PgExecutor<'e>,
    bill_run_id: &str,
    tenant_id: &str,
    subscriptions_processed: i32,
    invoices_created: i32,
    failures: i32,
    execution_time: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE bill_runs
         SET subscriptions_processed = $1,
             invoices_created = $2,
             failures = $3,
             status = 'completed',
             updated_at = $4
         WHERE bill_run_id = $5 AND tenant_id = $6",
    )
    .bind(subscriptions_processed)
    .bind(invoices_created)
    .bind(failures)
    .bind(execution_time)
    .bind(bill_run_id)
    .bind(tenant_id)
    .execute(executor)
    .await?;
    Ok(())
}

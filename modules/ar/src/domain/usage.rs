//! Usage repository — all SQL operations for metered usage.

use sqlx::PgExecutor;

use crate::models::UsageRecord;

/// Find an existing usage record by idempotency_key (no-op return).
pub async fn find_by_idempotency_key<'e>(
    executor: impl PgExecutor<'e>,
    idempotency_key: uuid::Uuid,
) -> Result<Option<UsageRecord>, sqlx::Error> {
    sqlx::query_as::<_, UsageRecord>(
        r#"
        SELECT id, usage_uuid, idempotency_key, app_id, customer_id, metric_name,
               quantity::float8 AS quantity, unit, unit_price_cents, period_start, period_end, recorded_at
        FROM ar_metered_usage
        WHERE idempotency_key = $1
        "#,
    )
    .bind(idempotency_key)
    .fetch_optional(executor)
    .await
}

/// Insert a new usage record. Must be called within a transaction.
#[allow(clippy::too_many_arguments)]
pub async fn insert_usage<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    customer_id: i32,
    metric_name: &str,
    quantity: f64,
    unit_price_cents: i32,
    period_start: chrono::NaiveDateTime,
    period_end: chrono::NaiveDateTime,
    idempotency_key: uuid::Uuid,
    unit: &str,
) -> Result<UsageRecord, sqlx::Error> {
    sqlx::query_as::<_, UsageRecord>(
        r#"
        INSERT INTO ar_metered_usage (
            app_id, customer_id, metric_name, quantity, unit_price_cents,
            period_start, period_end, recorded_at,
            idempotency_key, usage_uuid, unit
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), $8, gen_random_uuid(), $9)
        RETURNING id, usage_uuid, idempotency_key, app_id, customer_id, metric_name,
                  quantity::float8 AS quantity, unit, unit_price_cents, period_start, period_end, recorded_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(metric_name)
    .bind(quantity)
    .bind(unit_price_cents)
    .bind(period_start)
    .bind(period_end)
    .bind(idempotency_key)
    .bind(unit)
    .fetch_one(executor)
    .await
}

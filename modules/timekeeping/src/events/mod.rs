//! Timekeeping outbox — enqueue events atomically with mutations.

use serde::Serialize;
use uuid::Uuid;

/// Enqueue an event into the outbox within a caller-supplied transaction.
///
/// Follows the canonical Guard→Mutation→Outbox atomicity pattern.
pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: &T,
) -> Result<(), sqlx::Error> {
    let payload_json = serde_json::to_value(payload).map_err(|e| {
        sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize event payload: {}", e),
        )))
    })?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload_json)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Check idempotency key; returns cached response if replay.
pub async fn check_idempotency(
    pool: &sqlx::PgPool,
    app_id: &str,
    key: &str,
) -> Result<Option<(serde_json::Value, i32)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT response_body, status_code FROM tk_idempotency_keys \
         WHERE app_id = $1 AND idempotency_key = $2 LIMIT 1",
    )
    .bind(app_id)
    .bind(key)
    .fetch_optional(pool)
    .await
}

/// Record idempotency key within a transaction.
pub async fn record_idempotency<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    key: &str,
    response: &T,
    status_code: i32,
) -> Result<(), sqlx::Error> {
    let body = serde_json::to_value(response).unwrap_or(serde_json::Value::Null);
    let expires = chrono::Utc::now() + chrono::Duration::hours(24);

    sqlx::query(
        r#"
        INSERT INTO tk_idempotency_keys
            (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, '', $3, $4, $5)
        ON CONFLICT (app_id, idempotency_key) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(key)
    .bind(body)
    .bind(status_code)
    .bind(expires)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

//! Transactional outbox for integrations module.
//!
//! Enqueues events atomically within the caller's transaction into `integrations_outbox`.

mod relay;

pub use relay::{publish_batch, run_publisher_task, DEFAULT_MAX_RETRIES};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub app_id: String,
    pub payload: serde_json::Value,
    pub retry_count: i32,
}

/// Enqueue an event into the integrations outbox within a caller-supplied transaction.
///
/// This is the canonical path for Guard→Mutation→Outbox atomicity.
pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    app_id: &str,
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
        INSERT INTO integrations_outbox (
            event_id, event_type, aggregate_type, aggregate_id,
            app_id, payload, schema_version
        )
        VALUES ($1, $2, $3, $4, $5, $6, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(app_id)
    .bind(payload_json)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn fetch_unpublished(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    limit: i64,
) -> Result<Vec<OutboxEvent>, sqlx::Error> {
    sqlx::query_as::<_, OutboxEvent>(
        r#"
        SELECT event_id, event_type, aggregate_type, aggregate_id, app_id, payload, retry_count
        FROM integrations_outbox
        WHERE published_at IS NULL
          AND failed_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
}

/// A failed outbox row exposed via the DLQ read API.
/// `payload` is intentionally omitted — it may contain secrets.
#[derive(Debug, Serialize, Deserialize, FromRow, ToSchema)]
pub struct DlqEntry {
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub app_id: String,
    pub retry_count: i32,
    pub error_message: Option<String>,
    pub failure_reason: Option<String>,
    pub failed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// List failed outbox rows for a tenant, optionally filtered by `failure_reason`.
///
/// Returns at most `page_size` rows ordered by `failed_at DESC`.
pub async fn list_failed(
    pool: &sqlx::PgPool,
    app_id: &str,
    failure_reason: Option<&str>,
    page: i64,
    page_size: i64,
) -> Result<(Vec<DlqEntry>, i64), sqlx::Error> {
    let offset = (page - 1).max(0) * page_size;

    let rows = if let Some(reason) = failure_reason {
        sqlx::query_as::<_, DlqEntry>(
            r#"
            SELECT event_id, event_type, aggregate_type, aggregate_id, app_id,
                   retry_count, error_message, failure_reason, failed_at, created_at
            FROM integrations_outbox
            WHERE app_id = $1 AND failed_at IS NOT NULL AND failure_reason = $2
            ORDER BY failed_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(app_id)
        .bind(reason)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, DlqEntry>(
            r#"
            SELECT event_id, event_type, aggregate_type, aggregate_id, app_id,
                   retry_count, error_message, failure_reason, failed_at, created_at
            FROM integrations_outbox
            WHERE app_id = $1 AND failed_at IS NOT NULL
            ORDER BY failed_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?
    };

    let total: (i64,) = if let Some(reason) = failure_reason {
        sqlx::query_as(
            "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND failed_at IS NOT NULL AND failure_reason = $2",
        )
        .bind(app_id)
        .bind(reason)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND failed_at IS NOT NULL",
        )
        .bind(app_id)
        .fetch_one(pool)
        .await?
    };

    Ok((rows, total.0))
}

pub async fn mark_published(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrations_outbox
        SET published_at = NOW(), error_message = NULL
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

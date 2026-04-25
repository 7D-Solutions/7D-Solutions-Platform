mod publisher;

pub use publisher::run_publisher_task;

use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
}

/// Enqueue an event into the outbox within a caller-supplied transaction.
pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
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
        INSERT INTO sr_events_outbox (event_id, event_type, aggregate_type, aggregate_id, tenant_id, payload)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(payload_json)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Enqueue an event into the outbox using a pool (internally wraps in a transaction).
///
/// Use this when the caller does not already hold a transaction — e.g., webhook
/// handlers that emit a single event outside a larger business mutation.
pub async fn enqueue_event_tx_pool<T: Serialize>(
    pool: &sqlx::PgPool,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
    payload: &T,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    enqueue_event_tx(&mut tx, event_id, event_type, aggregate_type, aggregate_id, tenant_id, payload).await?;
    tx.commit().await
}

pub async fn fetch_unpublished(
    db: &sqlx::PgPool,
    limit: i64,
) -> Result<Vec<OutboxEvent>, sqlx::Error> {
    sqlx::query_as::<_, OutboxEvent>(
        r#"
        SELECT event_id, event_type, aggregate_type, aggregate_id, payload
        FROM sr_events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn mark_published(db: &sqlx::PgPool, event_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sr_events_outbox SET published_at = NOW() WHERE event_id = $1")
        .bind(event_id)
        .execute(db)
        .await?;
    Ok(())
}

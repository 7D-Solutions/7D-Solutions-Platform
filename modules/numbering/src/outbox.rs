//! Outbox pattern for reliable event publishing.

use serde::Serialize;
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
}

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

pub async fn fetch_unpublished(db: &PgPool, limit: i64) -> Result<Vec<OutboxEvent>, sqlx::Error> {
    sqlx::query_as::<_, OutboxEvent>(
        r#"
        SELECT event_id, event_type, aggregate_type, aggregate_id, payload
        FROM events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(db)
    .await
}

pub async fn mark_published(db: &PgPool, event_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE events_outbox SET published_at = NOW() WHERE event_id = $1")
        .bind(event_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn run_publisher_task(db: PgPool, event_bus: Arc<dyn event_bus::EventBus>) {
    tracing::info!("Numbering: starting outbox publisher task");

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut tick_count: u64 = 0;

    loop {
        interval.tick().await;
        tick_count += 1;

        match publish_batch(&db, &event_bus).await {
            Ok(n) if n > 0 => {
                tracing::info!("Numbering: outbox tick {}: published {} events", tick_count, n);
            }
            Ok(_) => {
                if tick_count <= 3 || tick_count.is_multiple_of(60) {
                    tracing::debug!("Numbering: outbox tick {}: no unpublished events", tick_count);
                }
            }
            Err(e) => {
                tracing::error!("Numbering: outbox tick {}: error: {}", tick_count, e);
            }
        }
    }
}

async fn publish_batch(
    db: &PgPool,
    event_bus: &Arc<dyn event_bus::EventBus>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let events = fetch_unpublished(db, 100).await?;
    let count = events.len();

    for event in events {
        let subject = event.event_type.clone();
        let payload = serde_json::to_vec(&event.payload)?;

        event_bus.publish(&subject, payload).await.map_err(|e| {
            tracing::error!(
                event_id = %event.event_id,
                subject = %subject,
                "Numbering: failed to publish event: {}", e
            );
            e
        })?;

        mark_published(db, event.event_id).await?;
    }

    Ok(count)
}

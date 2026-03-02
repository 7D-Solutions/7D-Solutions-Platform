use serde::Serialize;
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Outbox row as stored in events_outbox
#[derive(Debug, FromRow)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
}

/// Enqueue an event into the outbox within a caller-supplied transaction.
///
/// This is the canonical path for Guard→Mutation→Outbox atomicity.
/// The caller owns the transaction; this function only inserts into the outbox.
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

/// Fetch unpublished events from the outbox (oldest first, up to limit).
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

/// Mark an event as published.
pub async fn mark_published(db: &PgPool, event_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE events_outbox SET published_at = NOW() WHERE event_id = $1")
        .bind(event_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Background publisher loop: polls outbox and publishes to event bus.
///
/// Matches the platform convention from AR publisher — 1-second poll interval,
/// subjects namespaced as `ap.events.<event_type>`.
pub async fn run_publisher_task(db: PgPool, event_bus: Arc<dyn event_bus::EventBus>) {
    tracing::info!("AP: starting event publisher task");

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut tick_count: u64 = 0;

    loop {
        interval.tick().await;
        tick_count += 1;

        match publish_batch(&db, &event_bus).await {
            Ok(n) if n > 0 => {
                tracing::info!("AP publisher tick {}: published {} events", tick_count, n);
            }
            Ok(_) => {
                if tick_count <= 3 || tick_count.is_multiple_of(60) {
                    tracing::debug!("AP publisher tick {}: no unpublished events", tick_count);
                }
            }
            Err(e) => {
                tracing::error!("AP publisher tick {}: error: {}", tick_count, e);
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
        let subject = format!("ap.events.{}", event.event_type);
        let payload = serde_json::to_vec(&event.payload)?;

        event_bus.publish(&subject, payload).await.map_err(|e| {
            tracing::error!(
                event_id = %event.event_id,
                subject = %subject,
                error = %e,
                "AP: failed to publish event"
            );
            e
        })?;

        mark_published(db, event.event_id).await?;

        tracing::info!(
            event_id = %event.event_id,
            event_type = %event.event_type,
            subject = %subject,
            "AP: event published"
        );
    }

    Ok(count)
}

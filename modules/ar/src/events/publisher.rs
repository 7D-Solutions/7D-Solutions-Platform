use crate::events::outbox::{fetch_unpublished_events, mark_as_published};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

/// Background task that publishes events from the outbox to the event bus
///
/// This task polls the events_outbox table for unpublished events and publishes
/// them to the configured event bus (NATS or in-memory). Once published, events
/// are marked with a published_at timestamp.
pub async fn run_publisher_task(
    db: PgPool,
    event_bus: Arc<dyn event_bus::EventBus>,
) {
    tracing::info!("Starting event publisher task");

    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        match publish_batch(&db, &event_bus).await {
            Ok(count) if count > 0 => {
                tracing::debug!("Published {} events from outbox", count);
            }
            Ok(_) => {
                // No events to publish
            }
            Err(e) => {
                tracing::error!("Error publishing events: {}", e);
            }
        }
    }
}

async fn publish_batch(
    db: &PgPool,
    event_bus: &Arc<dyn event_bus::EventBus>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let events = fetch_unpublished_events(db, 100).await?;
    let count = events.len();

    for event in events {
        // Determine subject based on event type
        // Format: ar.events.<event_type>
        let subject = format!("ar.events.{}", event.event_type.replace('.', "."));

        // Serialize payload to bytes
        let payload = serde_json::to_vec(&event.payload)?;

        // Publish to event bus
        event_bus.publish(&subject, payload).await.map_err(|e| {
            tracing::error!(
                event_id = %event.event_id,
                subject = %subject,
                error = %e,
                "Failed to publish event"
            );
            e
        })?;

        // Mark as published
        mark_as_published(db, event.event_id).await?;

        tracing::info!(
            event_id = %event.event_id,
            event_type = %event.event_type,
            subject = %subject,
            "Event published successfully"
        );
    }

    Ok(count)
}

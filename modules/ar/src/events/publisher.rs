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
    let mut tick_count: u64 = 0;

    loop {
        interval.tick().await;
        tick_count += 1;

        match publish_batch(&db, &event_bus).await {
            Ok(count) if count > 0 => {
                tracing::info!("Publisher tick {}: published {} events from outbox", tick_count, count);
            }
            Ok(_) => {
                if tick_count <= 3 || tick_count % 60 == 0 {
                    tracing::info!("Publisher tick {}: no unpublished events", tick_count);
                }
            }
            Err(e) => {
                tracing::error!("Publisher tick {}: error publishing events: {}", tick_count, e);
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
        // For cross-module events (like GL posting), use the target module's namespace
        // Otherwise use AR's namespace
        let subject = if event.event_type.starts_with("gl.") {
            // GL events go directly to gl.events.* namespace
            format!("gl.events.{}", event.event_type.strip_prefix("gl.").unwrap_or(&event.event_type))
        } else {
            // AR events go to ar.events.* namespace
            format!("ar.events.{}", event.event_type.replace('.', "."))
        };

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

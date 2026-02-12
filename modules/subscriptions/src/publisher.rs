use crate::outbox::{fetch_unpublished_events, mark_as_published};
use event_bus::EventBus;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Background task that publishes events from the outbox to the event bus
pub async fn run_publisher(pool: PgPool, bus: Arc<dyn EventBus>) {
    tracing::info!("Starting event publisher task");

    loop {
        match publish_pending_events(&pool, &bus).await {
            Ok(count) => {
                if count > 0 {
                    tracing::debug!("Published {} events from outbox", count);
                }
            }
            Err(e) => {
                tracing::error!("Error publishing events: {}", e);
            }
        }

        // Poll every 1 second
        sleep(Duration::from_secs(1)).await;
    }
}

/// Publish all pending events from the outbox
async fn publish_pending_events(
    pool: &PgPool,
    bus: &Arc<dyn EventBus>,
) -> Result<usize, Box<dyn std::error::Error>> {
    // Fetch unpublished events (limit to 100 per batch)
    let events = fetch_unpublished_events(pool, 100).await?;

    let mut published_count = 0;

    for event in events {
        let event_id = event.id;

        // Construct NATS subject: subscriptions.events.<event_type>
        let nats_subject = format!("subscriptions.events.{}", event.subject);

        // Serialize envelope payload to bytes
        let payload_bytes = serde_json::to_vec(&event.payload)?;

        // Publish to event bus
        match bus.publish(&nats_subject, payload_bytes).await {
            Ok(_) => {
                // Mark as published in database
                mark_as_published(pool, event_id).await?;
                published_count += 1;
                tracing::trace!(
                    "Published event {} to subject {}",
                    event_id,
                    nats_subject
                );
            }
            Err(e) => {
                tracing::error!(
                    "Failed to publish event {} to subject {}: {}",
                    event_id,
                    nats_subject,
                    e
                );
                // Don't mark as published, will retry on next iteration
            }
        }
    }

    Ok(published_count)
}

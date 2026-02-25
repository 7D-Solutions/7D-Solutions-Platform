use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

use super::{fetch_unpublished, mark_published};

/// Background publisher loop: polls outbox and publishes to event bus.
///
/// Events are published on NATS subjects matching `event_type`.
/// The loop runs indefinitely; errors are logged and retried on the next tick.
pub async fn run_publisher_task(db: PgPool, event_bus: Arc<dyn event_bus::EventBus>) {
    tracing::info!("Shipping-Receiving: starting outbox publisher task");

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut tick_count: u64 = 0;

    loop {
        interval.tick().await;
        tick_count += 1;

        match publish_batch(&db, &event_bus).await {
            Ok(n) if n > 0 => {
                tracing::info!(
                    "Shipping-Receiving: outbox tick {}: published {} events",
                    tick_count,
                    n
                );
            }
            Ok(_) => {
                if tick_count <= 3 || tick_count % 60 == 0 {
                    tracing::debug!(
                        "Shipping-Receiving: outbox tick {}: no unpublished events",
                        tick_count
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    "Shipping-Receiving: outbox tick {}: error: {}",
                    tick_count,
                    e
                );
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
                error = %e,
                "Shipping-Receiving: failed to publish event"
            );
            e
        })?;

        mark_published(db, event.event_id).await?;

        tracing::info!(
            event_id = %event.event_id,
            event_type = %event.event_type,
            "Shipping-Receiving: event published"
        );
    }

    Ok(count)
}

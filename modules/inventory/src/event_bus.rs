use event_bus::EventBus;
use serde_json::Value;
use sqlx::PgPool;
use std::{error::Error, sync::Arc};
use tokio::time::{interval, Duration};
use uuid::Uuid;

/// Background publisher that drains `inv_outbox` and pushes envelopes to the bus.
pub async fn start_outbox_publisher(
    pool: PgPool,
    bus: Arc<dyn EventBus>,
) -> Result<(), Box<dyn Error>> {
    #[derive(sqlx::FromRow)]
    struct OutboxEvent {
        id: i64,
        event_id: Uuid,
        event_type: String,
        payload: Value,
    }

    let mut ticker = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        let events: Vec<OutboxEvent> = sqlx::query_as::<_, OutboxEvent>(
            r#"
            SELECT id, event_id, event_type, payload
            FROM inv_outbox
            WHERE published_at IS NULL
            ORDER BY created_at ASC
            LIMIT 100
            "#,
        )
        .fetch_all(&pool)
        .await?;

        if events.is_empty() {
            continue;
        }

        for event in events {
            let payload_bytes = serde_json::to_vec(&event.payload)?;
            let subject = format!("inventory.events.{}", event.event_type);

            match bus.publish(&subject, payload_bytes).await {
                Ok(_) => {
                    sqlx::query(
                        "UPDATE inv_outbox SET published_at = NOW() WHERE id = $1",
                    )
                    .bind(event.id)
                    .execute(&pool)
                    .await?;

                    tracing::info!(
                        event_id = %event.event_id,
                        event_type = %event.event_type,
                        "Published inventory outbox event"
                    );
                }
                Err(err) => {
                    tracing::error!(
                        event_id = %event.event_id,
                        event_type = %event.event_type,
                        error = %err,
                        "Failed to publish inventory outbox event"
                    );
                }
            }
        }
    }
}

/// Outbox relay for provisioning events.
///
/// Polls `provisioning_outbox` for unpublished rows and publishes each
/// event to the NATS bus using its `event_type` as the subject.  Marks
/// rows as published after successful delivery so they are not resent.
use event_bus::EventBus;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

pub async fn start_outbox_relay(pool: PgPool, bus: Arc<dyn EventBus>) {
    tracing::info!("provisioning outbox relay started");
    loop {
        match relay_batch(&pool, &bus).await {
            Ok(n) if n > 0 => {
                tracing::info!(published = n, "provisioning outbox relay: events published");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(error = %e, "provisioning outbox relay error");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn relay_batch(
    pool: &PgPool,
    bus: &Arc<dyn EventBus>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, OutboxRow>(
        "SELECT id, event_type, payload FROM provisioning_outbox \
         WHERE published_at IS NULL \
         ORDER BY created_at ASC \
         LIMIT 50 \
         FOR UPDATE SKIP LOCKED",
    )
    .fetch_all(pool)
    .await?;

    let count = rows.len();

    for row in rows {
        let payload_bytes = serde_json::to_vec(&row.payload)?;
        bus.publish(&row.event_type, payload_bytes).await?;

        sqlx::query("UPDATE provisioning_outbox SET published_at = now() WHERE id = $1")
            .bind(row.id)
            .execute(pool)
            .await?;

        tracing::debug!(id = %row.id, event_type = %row.event_type, "provisioning event published");
    }

    Ok(count)
}

#[derive(sqlx::FromRow)]
struct OutboxRow {
    id: uuid::Uuid,
    event_type: String,
    payload: serde_json::Value,
}

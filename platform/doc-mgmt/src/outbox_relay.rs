use event_bus::EventBus;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

pub async fn start_outbox_relay(pool: PgPool, bus: Arc<dyn EventBus>) {
    loop {
        if let Err(e) = relay_batch(&pool, &bus).await {
            tracing::error!(error = %e, "outbox relay error");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn relay_batch(
    pool: &PgPool,
    bus: &Arc<dyn EventBus>,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, OutboxRow>(
        "SELECT id, subject, payload FROM doc_outbox
         WHERE published_at IS NULL
         ORDER BY id ASC
         LIMIT 50
         FOR UPDATE SKIP LOCKED",
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        let payload_bytes = serde_json::to_vec(&row.payload)?;
        bus.publish(&row.subject, payload_bytes).await?;

        sqlx::query("UPDATE doc_outbox SET published_at = now() WHERE id = $1")
            .bind(row.id)
            .execute(pool)
            .await?;
    }

    Ok(())
}

#[derive(sqlx::FromRow)]
struct OutboxRow {
    id: i64,
    subject: String,
    payload: serde_json::Value,
}

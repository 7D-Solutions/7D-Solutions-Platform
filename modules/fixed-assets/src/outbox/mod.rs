use serde::Serialize;
use sqlx::{FromRow, PgPool};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Outbox row as stored in fa_events_outbox
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
/// Guard → Mutation → Outbox atomicity: caller owns the transaction;
/// this function only inserts into the outbox table.
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
            e.to_string(),
        )))
    })?;

    sqlx::query(
        r#"
        INSERT INTO fa_events_outbox
            (event_id, event_type, aggregate_type, aggregate_id, payload, created_at)
        VALUES ($1, $2, $3, $4, $5, NOW())
        ON CONFLICT (event_id) DO NOTHING
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

/// Background task: poll the outbox and publish pending events to the event bus.
///
/// Runs indefinitely; designed to be spawned with tokio::spawn.
pub async fn run_publisher_task(pool: PgPool, bus: Arc<dyn event_bus::EventBus>) {
    loop {
        match publish_pending(&pool, &bus).await {
            Ok(n) if n > 0 => tracing::debug!("Fixed-assets outbox: published {} events", n),
            Ok(_) => {}
            Err(e) => tracing::warn!("Fixed-assets outbox publisher error: {}", e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn publish_pending(
    pool: &PgPool,
    bus: &Arc<dyn event_bus::EventBus>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let rows: Vec<OutboxEvent> = sqlx::query_as(
        r#"
        SELECT event_id, event_type, aggregate_type, aggregate_id, payload
        FROM fa_events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    let count = rows.len();
    for row in rows {
        let subject = format!("{}.{}", row.aggregate_type, row.event_type);
        let data = serde_json::to_vec(&row.payload)?;
        bus.publish(&subject, data).await?;

        sqlx::query(
            "UPDATE fa_events_outbox SET published_at = NOW() WHERE event_id = $1",
        )
        .bind(row.event_id)
        .execute(pool)
        .await?;
    }

    Ok(count)
}

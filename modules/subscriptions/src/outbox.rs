use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

/// Outbox record for fetching unpublished events
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OutboxRecord {
    pub id: i64,
    pub subject: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

/// Enqueue an event to be published later
///
/// This function inserts an event into the events_outbox table for reliable delivery.
/// The background publisher will pick up these events and publish them to the event bus.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `event_type` - Event type for NATS subject routing (e.g., "billrun.completed")
/// * `envelope` - Platform-standard event envelope
pub async fn enqueue_event<T: Serialize>(
    pool: &PgPool,
    event_type: &str,
    envelope: &event_bus::EventEnvelope<T>,
) -> Result<i64, sqlx::Error> {
    // Serialize the entire envelope as payload
    let payload = serde_json::to_value(envelope)
        .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;

    let record = sqlx::query!(
        r#"
        INSERT INTO events_outbox (subject, payload)
        VALUES ($1, $2)
        RETURNING id
        "#,
        event_type,
        payload
    )
    .fetch_one(pool)
    .await?;

    tracing::debug!("Enqueued event {} to subject {}", record.id, event_type);

    Ok(record.id)
}

/// Fetch unpublished events from the outbox
pub async fn fetch_unpublished_events(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<OutboxRecord>, sqlx::Error> {
    let records = sqlx::query_as::<_, OutboxRecord>(
        r#"
        SELECT id, subject, payload, created_at, published_at
        FROM events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        "#
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(records)
}

/// Mark an event as published
pub async fn mark_as_published(pool: &PgPool, event_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        UPDATE events_outbox
        SET published_at = NOW()
        WHERE id = $1
        "#,
        event_id
    )
    .execute(pool)
    .await?;

    tracing::debug!("Marked event {} as published", event_id);

    Ok(())
}

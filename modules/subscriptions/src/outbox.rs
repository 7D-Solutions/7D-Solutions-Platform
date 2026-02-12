use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Event envelope for storing events in the outbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: Option<i64>,
    pub subject: String,
    pub payload: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
}

impl EventEnvelope {
    /// Create a new event envelope
    pub fn new(subject: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            id: None,
            subject: subject.into(),
            payload,
            created_at: None,
            published_at: None,
        }
    }
}

/// Enqueue an event to be published later
///
/// This function inserts an event into the events_outbox table for reliable delivery.
/// The background publisher will pick up these events and publish them to the event bus.
pub async fn enqueue_event(
    pool: &PgPool,
    subject: impl Into<String>,
    payload: serde_json::Value,
) -> Result<i64, sqlx::Error> {
    let subject = subject.into();

    let record = sqlx::query!(
        r#"
        INSERT INTO events_outbox (subject, payload)
        VALUES ($1, $2)
        RETURNING id
        "#,
        subject,
        payload
    )
    .fetch_one(pool)
    .await?;

    tracing::debug!("Enqueued event {} to subject {}", record.id, subject);

    Ok(record.id)
}

/// Fetch unpublished events from the outbox
pub async fn fetch_unpublished_events(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<EventEnvelope>, sqlx::Error> {
    let records = sqlx::query_as!(
        EventEnvelope,
        r#"
        SELECT id, subject, payload, created_at, published_at
        FROM events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        "#,
        limit
    )
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

use crate::events::envelope::EventEnvelope;
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Enqueue an event into the transactional outbox
///
/// This function stores the event in the database as part of the same transaction
/// as the business operation. A background publisher will pick it up and publish
/// to the event bus asynchronously.
pub async fn enqueue_event<T: Serialize>(
    db: &PgPool,
    envelope: &EventEnvelope<T>,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_value(&envelope)
        .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox
            (event_id, event_type, aggregate_type, aggregate_id, payload, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(envelope.event_id)
    .bind(&envelope.event_type)
    .bind(&envelope.aggregate_type)
    .bind(&envelope.aggregate_id)
    .bind(payload)
    .bind(envelope.occurred_at)
    .execute(db)
    .await?;

    tracing::debug!(
        event_id = %envelope.event_id,
        event_type = %envelope.event_type,
        "Event enqueued to outbox"
    );

    Ok(())
}

/// Fetch unpublished events from outbox (used by background publisher)
pub async fn fetch_unpublished_events(
    db: &PgPool,
    limit: i64,
) -> Result<Vec<UnpublishedEvent>, sqlx::Error> {
    let events = sqlx::query_as::<_, UnpublishedEvent>(
        r#"
        SELECT id, event_id, event_type, aggregate_type, aggregate_id, payload, created_at
        FROM events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(events)
}

/// Mark event as published in the outbox
pub async fn mark_as_published(
    db: &PgPool,
    event_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE events_outbox
        SET published_at = NOW()
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .execute(db)
    .await?;

    Ok(())
}

#[derive(Debug, FromRow)]
pub struct UnpublishedEvent {
    pub id: i32,
    pub event_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
    pub created_at: chrono::NaiveDateTime,
}

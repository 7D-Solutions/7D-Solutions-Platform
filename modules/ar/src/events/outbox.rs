use crate::events::envelope::EventEnvelope;
use event_bus::outbox::validate_and_serialize_envelope;
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Enqueue an event into the transactional outbox
///
/// This function stores the event in the database as part of the same transaction
/// as the business operation. A background publisher will pick it up and publish
/// to the event bus asynchronously.
///
/// **IMPORTANT**: This function enforces envelope validation at the boundary.
/// No event can be enqueued without passing constitutional validation.
///
/// # Arguments
/// * `db` - Database connection pool
/// * `event_type` - Event type for NATS subject routing (e.g., "payment.collection.requested")
/// * `aggregate_type` - Aggregate type for AR's DDD model (e.g., "invoice")
/// * `aggregate_id` - Aggregate instance ID
/// * `envelope` - Platform-standard event envelope
pub async fn enqueue_event<T: Serialize>(
    db: &PgPool,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    envelope: &EventEnvelope<T>,
) -> Result<(), sqlx::Error> {
    // Validate envelope at boundary - reject invalid envelopes before insert
    let payload = validate_and_serialize_envelope(envelope)
        .map_err(|e| sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Envelope validation failed: {}", e),
        ))))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, source_version, schema_version,
            occurred_at, replay_safe, trace_id, correlation_id, causation_id,
            reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        "#,
    )
    .bind(envelope.event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(&envelope.reverses_event_id)
    .bind(&envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(db)
    .await?;

    tracing::debug!(
        event_id = %envelope.event_id,
        event_type = %event_type,
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
        SELECT
            id, event_id, event_type, aggregate_type, aggregate_id, payload, created_at,
            tenant_id, source_module, source_version, schema_version,
            occurred_at, replay_safe, trace_id, correlation_id, causation_id,
            reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
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
    // Envelope metadata
    pub tenant_id: Option<String>,
    pub source_module: Option<String>,
    pub source_version: Option<String>,
    pub schema_version: Option<String>,
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    pub replay_safe: Option<bool>,
    pub trace_id: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub reverses_event_id: Option<Uuid>,
    pub supersedes_event_id: Option<Uuid>,
    pub side_effect_id: Option<String>,
    pub mutation_class: Option<String>,
}

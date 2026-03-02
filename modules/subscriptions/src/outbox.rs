use chrono::{DateTime, Utc};
use event_bus::outbox::validate_and_serialize_envelope;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Outbox record for fetching unpublished events
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OutboxRecord {
    pub id: i64,
    pub subject: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    // Envelope metadata
    pub event_id: Option<Uuid>,
    pub event_type: Option<String>,
    pub tenant_id: Option<String>,
    pub source_module: Option<String>,
    pub source_version: Option<String>,
    pub schema_version: Option<String>,
    pub replay_safe: Option<bool>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub trace_id: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub reverses_event_id: Option<Uuid>,
    pub supersedes_event_id: Option<Uuid>,
    pub side_effect_id: Option<String>,
    pub mutation_class: Option<String>,
}

/// Enqueue an event to be published later
///
/// This function inserts an event into the events_outbox table for reliable delivery.
/// The background publisher will pick up these events and publish them to the event bus.
///
/// **IMPORTANT**: This function enforces envelope validation at the boundary.
/// No event can be enqueued without passing constitutional validation.
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
    // Validate envelope at boundary - reject invalid envelopes before insert
    let payload = validate_and_serialize_envelope(envelope).map_err(|e| {
        sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Envelope validation failed: {}", e),
        )))
    })?;

    let record = sqlx::query!(
        r#"
        INSERT INTO events_outbox (
            subject, payload, event_id, event_type, tenant_id, source_module,
            source_version, schema_version, replay_safe, occurred_at,
            trace_id, correlation_id, causation_id, reverses_event_id,
            supersedes_event_id, side_effect_id, mutation_class
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        RETURNING id
        "#,
        event_type,
        payload,
        envelope.event_id,
        &envelope.event_type,
        &envelope.tenant_id,
        &envelope.source_module,
        &envelope.source_version,
        &envelope.schema_version,
        envelope.replay_safe,
        envelope.occurred_at,
        envelope.trace_id.as_ref(),
        envelope.correlation_id.as_ref(),
        envelope.causation_id.as_ref(),
        envelope.reverses_event_id.as_ref(),
        envelope.supersedes_event_id.as_ref(),
        envelope.side_effect_id.as_ref(),
        envelope.mutation_class.as_ref()
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
        SELECT
            id, subject, payload, created_at, published_at,
            event_id, event_type, tenant_id, source_module,
            source_version, schema_version, replay_safe, occurred_at,
            trace_id, correlation_id, causation_id, reverses_event_id,
            supersedes_event_id, side_effect_id, mutation_class
        FROM events_outbox
        WHERE published_at IS NULL
        ORDER BY created_at ASC
        LIMIT $1
        "#,
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

/// Transaction-aware version of enqueue_event for atomicity guarantees
///
/// This function enqueues an event within an existing transaction, ensuring
/// that the domain mutation and outbox insert commit atomically.
///
/// **Phase 16 Atomicity Fix (bd-299f):**
/// - Subscription mutations + outbox insert must be atomic
/// - Billing cycle advance + invoice intent events must be atomic
/// - Either BOTH succeed or BOTH rollback
/// - Prevents orphaned domain state without corresponding events
///
/// # Arguments
/// * `tx` - Active database transaction
/// * `event_type` - Event type for NATS subject routing
/// * `envelope` - Platform-standard event envelope
pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_type: &str,
    envelope: &event_bus::EventEnvelope<T>,
) -> Result<i64, sqlx::Error> {
    // Validate envelope at boundary - reject invalid envelopes before insert
    let payload = validate_and_serialize_envelope(envelope).map_err(|e| {
        sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Envelope validation failed: {}", e),
        )))
    })?;

    // Use manual query instead of query! macro for transaction support
    let record = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO events_outbox (
            subject, payload, event_id, event_type, tenant_id, source_module,
            source_version, schema_version, replay_safe, occurred_at,
            trace_id, correlation_id, causation_id, reverses_event_id,
            supersedes_event_id, side_effect_id, mutation_class
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        RETURNING id
        "#,
    )
    .bind(event_type)
    .bind(&payload)
    .bind(envelope.event_id)
    .bind(&envelope.event_type)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.replay_safe)
    .bind(envelope.occurred_at)
    .bind(envelope.trace_id.as_ref())
    .bind(envelope.correlation_id.as_ref())
    .bind(envelope.causation_id.as_ref())
    .bind(envelope.reverses_event_id.as_ref())
    .bind(envelope.supersedes_event_id.as_ref())
    .bind(envelope.side_effect_id.as_ref())
    .bind(envelope.mutation_class.as_ref())
    .fetch_one(&mut **tx)
    .await?;

    tracing::debug!(
        "Enqueued event {} to subject {} (in transaction)",
        record,
        event_type
    );

    Ok(record)
}

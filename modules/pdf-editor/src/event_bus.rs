use event_bus::outbox::validate_and_serialize_envelope;
use event_bus::EventBus;
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Re-export the platform-wide event envelope
pub use event_bus::EventEnvelope;

/// Helper function to create a pdf-editor-specific envelope
pub fn create_pdf_editor_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: Option<String>,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> EventEnvelope<T> {
    EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "pdf-editor".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(correlation_id.clone())
    .with_correlation_id(correlation_id)
    .with_causation_id(causation_id)
    .with_mutation_class(Some(mutation_class))
}

/// Enqueue an event for reliable publishing via the transactional outbox pattern
///
/// Writes the event to events_outbox within the same database transaction,
/// ensuring exactly-once delivery semantics. Enforces envelope validation at boundary.
pub async fn enqueue_event<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    subject: &str,
    envelope: &EventEnvelope<T>,
) -> Result<(), sqlx::Error> {
    let payload = validate_and_serialize_envelope(envelope).map_err(|e| {
        sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Envelope validation failed: {}", e),
        )))
    })?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, subject, payload, tenant_id, status,
            event_type, source_module, source_version, schema_version,
            occurred_at, replay_safe, trace_id, correlation_id, causation_id,
            reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
        )
        VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        "#
    )
    .bind(envelope.event_id)
    .bind(subject)
    .bind(payload)
    .bind(&envelope.tenant_id)
    .bind(&envelope.event_type)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(envelope.reverses_event_id)
    .bind(envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Background publisher task - polls outbox and publishes events to event bus
pub async fn start_outbox_publisher(db: PgPool, bus: Arc<dyn EventBus>) {
    info!("Starting pdf-editor outbox publisher task");

    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));

    loop {
        interval.tick().await;

        match publish_pending_events(&db, &bus).await {
            Ok(count) if count > 0 => {
                info!("Published {} events from outbox", count);
            }
            Err(e) => {
                error!("Error publishing events from outbox: {}", e);
            }
            _ => {}
        }
    }
}

async fn publish_pending_events(
    db: &PgPool,
    bus: &Arc<dyn EventBus>,
) -> Result<usize, Box<dyn std::error::Error>> {
    #[derive(sqlx::FromRow)]
    struct OutboxEvent {
        id: i64,
        event_id: Uuid,
        subject: String,
        payload: serde_json::Value,
        #[allow(dead_code)]
        tenant_id: String,
        retry_count: i32,
    }

    let events = sqlx::query_as::<_, OutboxEvent>(
        r#"
        SELECT id, event_id, subject, payload, tenant_id, retry_count
        FROM events_outbox
        WHERE status = 'pending'
        ORDER BY created_at ASC
        LIMIT 100
        "#,
    )
    .fetch_all(db)
    .await?;

    let mut published_count = 0;

    for event in events {
        let payload_bytes = serde_json::to_vec(&event.payload)?;

        match bus.publish(&event.subject, payload_bytes).await {
            Ok(_) => {
                sqlx::query(
                    r#"
                    UPDATE events_outbox
                    SET status = 'published', published_at = NOW()
                    WHERE id = $1
                    "#,
                )
                .bind(event.id)
                .execute(db)
                .await?;

                published_count += 1;
            }
            Err(e) => {
                let retry_count = event.retry_count + 1;
                let status = if retry_count >= 5 {
                    "failed"
                } else {
                    "pending"
                };

                sqlx::query(
                    r#"
                    UPDATE events_outbox
                    SET retry_count = $1, status = $2, error_message = $3
                    WHERE id = $4
                    "#,
                )
                .bind(retry_count)
                .bind(status)
                .bind(e.to_string())
                .bind(event.id)
                .execute(db)
                .await?;

                warn!(
                    "Failed to publish event {} (retry {}/5): {}",
                    event.event_id, retry_count, e
                );
            }
        }
    }

    Ok(published_count)
}

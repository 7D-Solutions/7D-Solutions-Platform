use serde::Serialize;
use sqlx::PgPool;

use super::envelope::EventEnvelope;

/// Enqueue an event to the outbox for reliable publishing
///
/// Events in the outbox will be picked up by the background publisher
/// and sent to the event bus asynchronously.
pub async fn enqueue_event<T: Serialize>(
    pool: &PgPool,
    event_type: &str,
    envelope: &EventEnvelope<T>,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_value(&envelope.payload)
        .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;

    sqlx::query(
        r#"
        INSERT INTO payments_events_outbox (
            event_id,
            event_type,
            occurred_at,
            tenant_id,
            correlation_id,
            causation_id,
            payload,
            source_module,
            source_version,
            schema_version,
            replay_safe,
            trace_id,
            reverses_event_id,
            supersedes_event_id,
            side_effect_id,
            mutation_class
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        "#,
    )
    .bind(envelope.event_id)
    .bind(event_type)
    .bind(envelope.occurred_at)
    .bind(&envelope.tenant_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(payload)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.reverses_event_id)
    .bind(&envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(pool)
    .await?;

    tracing::debug!(
        event_id = %envelope.event_id,
        event_type = %event_type,
        "Event enqueued to outbox"
    );

    Ok(())
}

/// Background task that publishes events from outbox to the event bus
///
/// Polls the outbox table for unpublished events and sends them to the bus.
/// Marks events as published after successful transmission.
pub async fn start_outbox_publisher(
    pool: PgPool,
    bus: std::sync::Arc<dyn event_bus::EventBus>,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::time::{interval, Duration};

    let mut ticker = interval(Duration::from_secs(1));

    loop {
        ticker.tick().await;

        // Fetch unpublished events
        #[derive(sqlx::FromRow)]
        struct OutboxEvent {
            id: i32,
            event_id: uuid::Uuid,
            event_type: String,
            occurred_at: chrono::NaiveDateTime,
            tenant_id: String,
            correlation_id: Option<String>,
            causation_id: Option<String>,
            payload: serde_json::Value,
            source_module: Option<String>,
            source_version: Option<String>,
            schema_version: Option<String>,
            replay_safe: Option<bool>,
            trace_id: Option<String>,
            reverses_event_id: Option<uuid::Uuid>,
            supersedes_event_id: Option<uuid::Uuid>,
            side_effect_id: Option<String>,
            mutation_class: Option<String>,
        }

        let events: Vec<OutboxEvent> = sqlx::query_as(
            r#"
            SELECT id, event_id, event_type, occurred_at, tenant_id,
                   correlation_id, causation_id, payload,
                   source_module, source_version, schema_version, replay_safe,
                   trace_id, reverses_event_id, supersedes_event_id,
                   side_effect_id, mutation_class
            FROM payments_events_outbox
            WHERE published_at IS NULL
            ORDER BY occurred_at ASC
            LIMIT 100
            "#,
        )
        .fetch_all(&pool)
        .await?;

        if events.is_empty() {
            continue;
        }

        tracing::debug!("Publishing {} events from outbox", events.len());

        for event in events {
            // Construct full envelope for publishing
            let full_envelope = serde_json::json!({
                "event_id": event.event_id,
                "event_type": event.event_type,
                "occurred_at": event.occurred_at.and_utc().to_rfc3339(),
                "tenant_id": event.tenant_id,
                "source_module": event.source_module.unwrap_or_else(|| "payments".to_string()),
                "source_version": event.source_version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
                "schema_version": event.schema_version.unwrap_or_else(|| "1.0.0".to_string()),
                "replay_safe": event.replay_safe.unwrap_or(true),
                "correlation_id": event.correlation_id,
                "causation_id": event.causation_id,
                "trace_id": event.trace_id,
                "reverses_event_id": event.reverses_event_id,
                "supersedes_event_id": event.supersedes_event_id,
                "side_effect_id": event.side_effect_id,
                "mutation_class": event.mutation_class,
                "payload": event.payload,
            });

            let subject = format!("payments.events.{}", event.event_type.replace('.', "."));
            let payload_bytes = serde_json::to_vec(&full_envelope)?;

            // Publish to bus
            if let Err(e) = bus.publish(&subject, payload_bytes).await {
                tracing::error!(
                    event_id = %event.event_id,
                    error = %e,
                    "Failed to publish event"
                );
                continue;
            }

            // Mark as published
            sqlx::query(
                r#"
                UPDATE payments_events_outbox
                SET published_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(event.id)
            .execute(&pool)
            .await?;

            tracing::info!(
                event_id = %event.event_id,
                event_type = %event.event_type,
                "Event published successfully"
            );
        }
    }
}

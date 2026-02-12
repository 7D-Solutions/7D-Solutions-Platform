use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// EventEnvelope - Standard event wrapper following platform event contract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub tenant_id: String,
    pub source_module: String,
    pub source_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    pub payload: serde_json::Value,
}

impl EventEnvelope {
    /// Create a new event envelope
    pub fn new(tenant_id: String, payload: serde_json::Value) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            tenant_id,
            source_module: "notifications".to_string(),
            source_version: env!("CARGO_PKG_VERSION").to_string(),
            correlation_id: None,
            causation_id: None,
            payload,
        }
    }

    /// Set correlation ID for tracking related events
    pub fn with_correlation_id(mut self, correlation_id: String) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Set causation ID for event that triggered this one
    pub fn with_causation_id(mut self, causation_id: String) -> Self {
        self.causation_id = Some(causation_id);
        self
    }
}

/// Enqueue an event for reliable publishing via the transactional outbox pattern
///
/// This function writes the event to the events_outbox table within the same
/// database transaction, ensuring exactly-once delivery semantics.
pub async fn enqueue_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    subject: &str,
    envelope: &EventEnvelope,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_value(envelope)
        .map_err(|e| sqlx::Error::Encode(Box::new(e)))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (event_id, subject, payload, tenant_id, status)
        VALUES ($1, $2, $3, $4, 'pending')
        "#
    )
    .bind(envelope.event_id)
    .bind(subject)
    .bind(payload)
    .bind(&envelope.tenant_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Background publisher task - polls outbox and publishes events to event bus
///
/// This task runs continuously and publishes pending events from the outbox table
/// to the event bus. It handles retries and marks events as published.
pub async fn start_outbox_publisher(db: PgPool, bus: Arc<dyn EventBus>) {
    info!("Starting outbox publisher task");

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
        tenant_id: String,
        retry_count: i32,
    }

    // Fetch pending events (limit batch size to avoid overwhelming the bus)
    let events = sqlx::query_as::<_, OutboxEvent>(
        r#"
        SELECT id, event_id, subject, payload, tenant_id, retry_count
        FROM events_outbox
        WHERE status = 'pending'
        ORDER BY created_at ASC
        LIMIT 100
        "#
    )
    .fetch_all(db)
    .await?;

    let mut published_count = 0;

    for event in events {
        let payload_bytes = serde_json::to_vec(&event.payload)?;

        match bus.publish(&event.subject, payload_bytes).await {
            Ok(_) => {
                // Mark as published
                sqlx::query(
                    r#"
                    UPDATE events_outbox
                    SET status = 'published', published_at = NOW()
                    WHERE id = $1
                    "#
                )
                .bind(event.id)
                .execute(db)
                .await?;

                published_count += 1;
            }
            Err(e) => {
                // Increment retry count
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
                    "#
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

/// Check if an event has already been processed (idempotency)
pub async fn is_event_processed(db: &PgPool, event_id: Uuid) -> Result<bool, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct EventIdRow {
        event_id: Uuid,
    }

    let result = sqlx::query_as::<_, EventIdRow>(
        r#"
        SELECT event_id FROM processed_events WHERE event_id = $1
        "#
    )
    .bind(event_id)
    .fetch_optional(db)
    .await?;

    Ok(result.is_some())
}

/// Mark an event as processed (idempotency tracking)
pub async fn mark_event_processed(
    db: &PgPool,
    event_id: Uuid,
    subject: &str,
    tenant_id: &str,
    source_module: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO processed_events (event_id, subject, tenant_id, source_module)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (event_id) DO NOTHING
        "#
    )
    .bind(event_id)
    .bind(subject)
    .bind(tenant_id)
    .bind(source_module)
    .execute(db)
    .await?;

    Ok(())
}

/// Idempotent event consumer - wraps event processing with duplicate detection
///
/// This function ensures that events are processed exactly once, even if they
/// are delivered multiple times by the event bus.
pub async fn consume_event_idempotent<F, Fut>(
    db: &PgPool,
    msg: &BusMessage,
    handler: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce(EventEnvelope) -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    // Deserialize envelope
    let envelope: EventEnvelope = serde_json::from_slice(&msg.payload)?;

    // Check if already processed
    if is_event_processed(db, envelope.event_id).await? {
        info!("Event {} already processed, skipping", envelope.event_id);
        return Ok(());
    }

    // Process the event
    handler(envelope.clone()).await?;

    // Mark as processed
    mark_event_processed(
        db,
        envelope.event_id,
        &msg.subject,
        &envelope.tenant_id,
        &envelope.source_module,
    )
    .await?;

    info!("Event {} processed successfully", envelope.event_id);

    Ok(())
}

/// Start consuming events from a subject pattern
///
/// This spawns a background task that listens for events matching the subject
/// pattern and processes them using the provided handler.
pub async fn start_event_consumer<F, Fut>(
    bus: Arc<dyn EventBus>,
    db: PgPool,
    subject: &str,
    handler: F,
) where
    F: Fn(EventEnvelope) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>> + Send,
{
    let subject = subject.to_string();

    tokio::spawn(async move {
        info!("Starting event consumer for subject: {}", subject);

        let mut stream = match bus.subscribe(&subject).await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        while let Some(msg) = stream.next().await {
            match consume_event_idempotent(&db, &msg, |envelope| handler(envelope)).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Error processing event from {}: {}", msg.subject, e);
                }
            }
        }

        warn!("Event consumer for {} stopped", subject);
    });
}

//! GL Posting Request Consumer
//!
//! This consumer subscribes to `gl.events.posting.requested` and creates journal entries.

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::contracts::gl_posting_request_v1::GlPostingRequestV1;
use crate::services::journal_service::{process_gl_posting_request, JournalError};

/// Start the GL posting consumer task
///
/// This function spawns a background task that:
/// 1. Subscribes to gl.events.posting.requested
/// 2. Validates and processes GL posting requests
/// 3. Creates journal entries with idempotency
/// 4. Sends failed events to DLQ after retries
pub async fn start_gl_posting_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL posting consumer");

        // Subscribe to GL posting events
        let subject = "gl.events.posting.requested";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        tracing::info!("Subscribed to {}", subject);

        // Configure retry behavior: 3 attempts with exponential backoff
        let retry_config = RetryConfig::default();

        while let Some(msg) = stream.next().await {
            // Extract correlation fields from envelope for observability
            let (event_id, tenant_id, correlation_id, source_module) =
                match extract_correlation_fields(&msg) {
                    Ok(fields) => fields,
                    Err(e) => {
                        tracing::error!(
                            subject = %msg.subject,
                            error = %e,
                            "Failed to extract correlation fields from envelope"
                        );
                        continue;
                    }
                };

            // Create tracing span with correlation fields
            let span = tracing::info_span!(
                "process_gl_posting",
                event_id = %event_id,
                subject = %msg.subject,
                tenant_id = %tenant_id,
                correlation_id = %correlation_id.as_deref().unwrap_or("none"),
                source_module = %source_module.as_deref().unwrap_or("unknown")
            );

            // Process message within the span
            async {
                // Clone necessary data for retry closure
                let pool_clone = pool.clone();
                let msg_clone = msg.clone();

                // Determine if error is retriable or should go straight to DLQ
                let result = retry_with_backoff(
                    || {
                        let pool = pool_clone.clone();
                        let msg = msg_clone.clone();
                        async move {
                            process_gl_posting_message(&pool, &msg)
                                .await
                                .map_err(|e| format_error_for_retry(e))
                        }
                    },
                    &retry_config,
                    "gl_posting_consumer",
                )
                .await;

                // If all retries failed, send to DLQ
                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        retry_count = retry_config.max_attempts,
                        "Event processing failed after retries, sending to DLQ"
                    );

                    crate::dlq::handle_processing_error(
                        &pool,
                        &msg,
                        &error_msg,
                        retry_config.max_attempts as i32,
                    )
                    .await;
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("GL posting consumer stopped");
    });
}

/// Process a GL posting message
async fn process_gl_posting_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    // Parse the event envelope
    let envelope: EventEnvelope<GlPostingRequestV1> =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        source_module = %envelope.source_module,
        "Processing GL posting request"
    );

    // Process the posting request
    match process_gl_posting_request(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.source_module,
        &msg.subject,
        &envelope.payload,
    )
    .await
    {
        Ok(entry_id) => {
            tracing::info!(
                event_id = %envelope.event_id,
                entry_id = %entry_id,
                "Successfully created journal entry"
            );
            Ok(())
        }
        Err(JournalError::Validation(e)) => {
            // Validation errors are not retriable - go straight to DLQ
            Err(ProcessingError::Validation(format!(
                "Validation failed: {}",
                e
            )))
        }
        Err(JournalError::InvalidDate(e)) => {
            // Date parsing errors are not retriable
            Err(ProcessingError::Validation(format!("Invalid date: {}", e)))
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            // Duplicate events are expected (idempotency) - not an error
            tracing::info!(
                event_id = %event_id,
                "Duplicate event ignored (already processed)"
            );
            Ok(())
        }
        Err(JournalError::Database(e)) => {
            // Database errors are retriable
            Err(ProcessingError::Retriable(format!("Database error: {}", e)))
        }
    }
}

/// Extract correlation fields from event envelope for observability
///
/// Returns: (event_id, tenant_id, correlation_id, source_module)
fn extract_correlation_fields(
    msg: &BusMessage,
) -> Result<(Uuid, String, Option<String>, Option<String>), Box<dyn std::error::Error>> {
    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;

    let event_id_str = envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing event_id")?;
    let event_id = Uuid::parse_str(event_id_str)?;

    let tenant_id = envelope
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing tenant_id")?
        .to_string();

    let correlation_id = envelope
        .get("correlation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let source_module = envelope
        .get("source_module")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((event_id, tenant_id, correlation_id, source_module))
}

/// Error types for processing GL posting requests
#[derive(Debug)]
enum ProcessingError {
    /// Validation errors are not retriable (send to DLQ immediately)
    Validation(String),
    /// Retriable errors (database, network, etc.)
    Retriable(String),
}

impl std::fmt::Display for ProcessingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessingError::Validation(msg) => write!(f, "Validation error: {}", msg),
            ProcessingError::Retriable(msg) => write!(f, "Retriable error: {}", msg),
        }
    }
}

/// Format error for retry logic
///
/// Validation errors return immediately (not retriable).
/// Retriable errors continue with retry logic.
fn format_error_for_retry(error: ProcessingError) -> String {
    match error {
        ProcessingError::Validation(msg) => {
            // For validation errors, we want to fail immediately without retries
            // The retry_with_backoff will not retry if we return this error
            format!("[NON_RETRIABLE] {}", msg)
        }
        ProcessingError::Retriable(msg) => {
            // Retriable errors will go through exponential backoff
            format!("[RETRIABLE] {}", msg)
        }
    }
}

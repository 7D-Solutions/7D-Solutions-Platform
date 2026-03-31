//! GL Inventory Consumer — NATS wiring for inventory GL postings
//!
//! Subscribes to inventory events and delegates to posting functions
//! in `gl_inventory_posting` for journal entry construction.

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::services::journal_service::JournalError;

// Re-export posting types and functions for backward compatibility
pub use super::gl_inventory_posting::{
    process_inventory_cogs_posting, process_inventory_wip_posting,
    process_production_receipt_posting, ConsumedLayer, ItemIssuedPayload, ItemReceivedPayload,
    SourceRef, SOURCE_TYPE_PRODUCTION, SOURCE_TYPE_PURCHASE, SOURCE_TYPE_SALES_ORDER,
};

/// Start the GL inventory consumer tasks.
///
/// Subscribes to:
/// - `inventory.item_issued` — branches on source_type for COGS vs WIP
/// - `inventory.item_received` — handles production receipts (FG at rolled-up cost)
pub async fn start_gl_inventory_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    let bus_issued = bus.clone();
    let pool_issued = pool.clone();
    tokio::spawn(async move {
        start_item_issued_consumer(bus_issued, pool_issued).await;
    });

    tokio::spawn(async move {
        start_item_received_consumer(bus, pool).await;
    });
}

async fn start_item_issued_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tracing::info!("Starting GL inventory item_issued consumer");

    let subject = "inventory.item_issued";
    let mut stream = match bus.subscribe(subject).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to subscribe to {}: {}", subject, e);
            return;
        }
    };

    tracing::info!("Subscribed to {}", subject);
    let retry_config = RetryConfig::default();

    while let Some(msg) = stream.next().await {
        let (event_id, tenant_id, correlation_id, source_module) =
            match extract_correlation_fields(&msg) {
                Ok(fields) => fields,
                Err(e) => {
                    tracing::error!(
                        subject = %msg.subject,
                        error = %e,
                        "Failed to extract correlation fields"
                    );
                    continue;
                }
            };

        let span = tracing::info_span!(
            "process_inventory_gl_posting",
            event_id = %event_id,
            tenant_id = %tenant_id,
            correlation_id = %correlation_id.as_deref().unwrap_or("none"),
            source_module = %source_module.as_deref().unwrap_or("unknown")
        );

        async {
            let pool_clone = pool.clone();
            let msg_clone = msg.clone();

            let result = retry_with_backoff(
                || {
                    let pool = pool_clone.clone();
                    let msg = msg_clone.clone();
                    async move {
                        process_item_issued_message(&pool, &msg)
                            .await
                            .map_err(format_error_for_retry)
                    }
                },
                &retry_config,
                "gl_inventory_consumer",
            )
            .await;

            if let Err(error_msg) = result {
                tracing::error!(
                    error = %error_msg,
                    "Inventory GL posting failed after retries, sending to DLQ"
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

    tracing::warn!("GL inventory item_issued consumer stopped");
}

async fn start_item_received_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tracing::info!("Starting GL inventory item_received consumer");

    let subject = "inventory.item_received";
    let mut stream = match bus.subscribe(subject).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to subscribe to {}: {}", subject, e);
            return;
        }
    };

    tracing::info!("Subscribed to {}", subject);
    let retry_config = RetryConfig::default();

    while let Some(msg) = stream.next().await {
        let (event_id, tenant_id, correlation_id, source_module) =
            match extract_correlation_fields(&msg) {
                Ok(fields) => fields,
                Err(e) => {
                    tracing::error!(
                        subject = %msg.subject,
                        error = %e,
                        "Failed to extract correlation fields"
                    );
                    continue;
                }
            };

        let span = tracing::info_span!(
            "process_inventory_received_gl_posting",
            event_id = %event_id,
            tenant_id = %tenant_id,
            correlation_id = %correlation_id.as_deref().unwrap_or("none"),
            source_module = %source_module.as_deref().unwrap_or("unknown")
        );

        async {
            let pool_clone = pool.clone();
            let msg_clone = msg.clone();

            let result = retry_with_backoff(
                || {
                    let pool = pool_clone.clone();
                    let msg = msg_clone.clone();
                    async move {
                        process_item_received_message(&pool, &msg)
                            .await
                            .map_err(format_error_for_retry)
                    }
                },
                &retry_config,
                "gl_inventory_received_consumer",
            )
            .await;

            if let Err(error_msg) = result {
                tracing::error!(
                    error = %error_msg,
                    "Inventory received GL posting failed after retries, sending to DLQ"
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

    tracing::warn!("GL inventory item_received consumer stopped");
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_item_issued_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<ItemIssuedPayload> =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse item_issued envelope: {}", e))
        })?;

    let source_type = &envelope.payload.source_ref.source_type;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        item_id = %envelope.payload.item_id,
        sku = %envelope.payload.sku,
        quantity = %envelope.payload.quantity,
        total_cost_minor = %envelope.payload.total_cost_minor,
        source_type = %source_type,
        "Processing inventory GL posting"
    );

    let result = match source_type.as_str() {
        SOURCE_TYPE_PURCHASE | SOURCE_TYPE_SALES_ORDER => {
            process_inventory_cogs_posting(
                pool,
                envelope.event_id,
                &envelope.tenant_id,
                &envelope.source_module,
                &envelope.payload,
            )
            .await
        }
        SOURCE_TYPE_PRODUCTION => {
            process_inventory_wip_posting(
                pool,
                envelope.event_id,
                &envelope.tenant_id,
                &envelope.source_module,
                &envelope.payload,
            )
            .await
        }
        unknown => {
            return Err(ProcessingError::Validation(format!(
                "Unknown source_type '{}' on item_issued event {} — cannot determine GL path",
                unknown, envelope.event_id
            )));
        }
    };

    match result {
        Ok(entry_id) => {
            tracing::info!(
                event_id = %envelope.event_id,
                entry_id = %entry_id,
                source_type = %source_type,
                "Inventory GL journal entry created"
            );
            Ok(())
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            tracing::info!(event_id = %event_id, "Duplicate item_issued event ignored");
            Ok(())
        }
        Err(JournalError::Validation(e)) => {
            Err(ProcessingError::Validation(format!("Validation: {}", e)))
        }
        Err(JournalError::InvalidDate(e)) => {
            Err(ProcessingError::Validation(format!("Invalid date: {}", e)))
        }
        Err(JournalError::Period(e)) => {
            Err(ProcessingError::Validation(format!("Period error: {}", e)))
        }
        Err(JournalError::Balance(e)) => {
            Err(ProcessingError::Retriable(format!("Balance error: {}", e)))
        }
        Err(JournalError::Database(e)) => {
            Err(ProcessingError::Retriable(format!("Database error: {}", e)))
        }
    }
}

async fn process_item_received_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<ItemReceivedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse item_received envelope: {}", e))
        })?;

    let source_type = &envelope.payload.source_type;

    match source_type.as_str() {
        SOURCE_TYPE_PRODUCTION => {
            tracing::info!(
                event_id = %envelope.event_id,
                tenant_id = %envelope.tenant_id,
                item_id = %envelope.payload.item_id,
                sku = %envelope.payload.sku,
                quantity = %envelope.payload.quantity,
                unit_cost_minor = %envelope.payload.unit_cost_minor,
                "Processing production receipt GL posting (FG at rolled-up cost)"
            );

            match process_production_receipt_posting(
                pool,
                envelope.event_id,
                &envelope.tenant_id,
                &envelope.source_module,
                &envelope.payload,
            )
            .await
            {
                Ok(entry_id) => {
                    tracing::info!(
                        event_id = %envelope.event_id,
                        entry_id = %entry_id,
                        "Production receipt GL journal entry created"
                    );
                    Ok(())
                }
                Err(JournalError::DuplicateEvent(event_id)) => {
                    tracing::info!(
                        event_id = %event_id,
                        "Duplicate item_received event ignored"
                    );
                    Ok(())
                }
                Err(JournalError::Validation(e)) => {
                    Err(ProcessingError::Validation(format!("Validation: {}", e)))
                }
                Err(JournalError::InvalidDate(e)) => {
                    Err(ProcessingError::Validation(format!("Invalid date: {}", e)))
                }
                Err(JournalError::Period(e)) => {
                    Err(ProcessingError::Validation(format!("Period error: {}", e)))
                }
                Err(JournalError::Balance(e)) => {
                    Err(ProcessingError::Retriable(format!("Balance error: {}", e)))
                }
                Err(JournalError::Database(e)) => {
                    Err(ProcessingError::Retriable(format!("Database error: {}", e)))
                }
            }
        }
        SOURCE_TYPE_PURCHASE => {
            tracing::debug!(
                event_id = %envelope.event_id,
                "Skipping purchase receipt — GL handled by AP module"
            );
            Ok(())
        }
        unknown => Err(ProcessingError::Validation(format!(
            "Unknown source_type '{}' on item_received event {} — cannot determine GL path",
            unknown, envelope.event_id
        ))),
    }
}

fn extract_correlation_fields(
    msg: &BusMessage,
) -> Result<(Uuid, String, Option<String>, Option<String>), Box<dyn std::error::Error>> {
    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
    let event_id = Uuid::parse_str(
        envelope
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing event_id")?,
    )?;
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

#[derive(Debug)]
enum ProcessingError {
    Validation(String),
    Retriable(String),
}

impl std::fmt::Display for ProcessingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(m) => write!(f, "Validation error: {}", m),
            Self::Retriable(m) => write!(f, "Retriable error: {}", m),
        }
    }
}

fn format_error_for_retry(error: ProcessingError) -> String {
    match error {
        ProcessingError::Validation(m) => format!("[NON_RETRIABLE] {}", m),
        ProcessingError::Retriable(m) => format!("[RETRIABLE] {}", m),
    }
}

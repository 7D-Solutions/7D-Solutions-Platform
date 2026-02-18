//! GL Inventory COGS Consumer (bd-1121)
//!
//! Handles `inventory.item_issued` events and posts balanced COGS journal entries to GL.
//!
//! ## Accounting Entry
//!
//! ```text
//! DR  COGS      total_cost_minor / 100.0   ← cost of goods recognized
//! CR  INVENTORY total_cost_minor / 100.0   ← inventory asset reduced
//! ```
//!
//! ## Idempotency
//! Uses `processed_events` table via `process_gl_posting_request`. The event_id from
//! the incoming envelope is the idempotency key — duplicate events are silently skipped.
//!
//! ## Period Validation
//! `process_gl_posting_request` enforces period existence and open/closed state.
//! Events targeting a closed period return `JournalError::Period` (non-retriable → DLQ).

use chrono::{DateTime, Utc};
use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::contracts::gl_posting_request_v1::{
    GlPostingRequestV1, JournalLine, SourceDocType,
};
use crate::services::journal_service::{process_gl_posting_request, JournalError};

// ============================================================================
// Inventory item_issued payload (mirrors inventory::events::contracts::ItemIssuedPayload)
// Deserialized from the NATS envelope payload field.
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConsumedLayer {
    pub layer_id: Uuid,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub extended_cost_minor: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SourceRef {
    pub source_module: String,
    pub source_type: String,
    pub source_id: String,
    pub source_line_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ItemIssuedPayload {
    pub issue_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub total_cost_minor: i64,
    pub currency: String,
    pub consumed_layers: Vec<ConsumedLayer>,
    pub source_ref: SourceRef,
    pub issued_at: DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process an item_issued event and post the balanced COGS GL journal entry.
///
/// Returns the created journal entry ID on success, or `JournalError` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
///
/// ## Journal entry
///   DR  COGS      — cost of goods recognized (inventory consumed)
///   CR  INVENTORY — inventory asset balance reduced
pub async fn process_inventory_cogs_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &ItemIssuedPayload,
) -> Result<Uuid, JournalError> {
    // Convert minor units to major units (cents → dollars)
    let amount = payload.total_cost_minor as f64 / 100.0;

    let posting = GlPostingRequestV1 {
        posting_date: payload.issued_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::InventoryIssue,
        source_doc_id: payload.issue_line_id.to_string(),
        description: format!(
            "COGS — issued {} units of {} ({})",
            payload.quantity, payload.sku, payload.source_ref.source_id
        ),
        lines: vec![
            JournalLine {
                account_ref: "COGS".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Cost of goods sold — {} units SKU {}",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
            JournalLine {
                account_ref: "INVENTORY".to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Inventory reduction — issued {} units SKU {}",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
        ],
    };

    let subject = format!("inventory.item_issued.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the GL inventory COGS consumer task.
///
/// Subscribes to `inventory.item_issued` NATS subject and posts COGS GL journal
/// entries via `process_inventory_cogs_posting`.
pub async fn start_gl_inventory_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL inventory COGS consumer");

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
                "process_inventory_cogs_posting",
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
                        "Inventory COGS GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL inventory COGS consumer stopped");
    });
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

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        item_id = %envelope.payload.item_id,
        sku = %envelope.payload.sku,
        quantity = %envelope.payload.quantity,
        total_cost_minor = %envelope.payload.total_cost_minor,
        "Processing inventory COGS GL posting"
    );

    match process_inventory_cogs_posting(
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
                "Inventory COGS GL journal entry created"
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

fn extract_correlation_fields(
    msg: &BusMessage,
) -> Result<(Uuid, String, Option<String>, Option<String>), Box<dyn std::error::Error>> {
    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
    let event_id = Uuid::parse_str(
        envelope.get("event_id").and_then(|v| v.as_str()).ok_or("Missing event_id")?,
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

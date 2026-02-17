//! GL Write-off Posting Consumer (bd-1rp)
//!
//! Handles `ar.invoice_written_off` events and posts balanced journal entries to GL.
//!
//! ## Accounting Entry (direct write-off method)
//!
//! ```text
//! DR  Bad Debt Expense ("BAD_DEBT")  amount_minor / 100.0   ← expense recognized
//! CR  Accounts Receivable ("AR")     amount_minor / 100.0   ← AR balance reduced
//! ```
//!
//! ## Idempotency
//! Uses `processed_events` table via `process_gl_posting_request`. The event_id from
//! the incoming envelope is the idempotency key — duplicate events are silently skipped.
//!
//! ## Period Validation
//! `process_gl_posting_request` enforces period existence and open/closed state.
//! Events targeting a closed period return `JournalError::Period` (non-retriable → DLQ).

use chrono::Utc;
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
// AR write-off payload (mirrors ar::events::contracts::InvoiceWrittenOffPayload)
// Deserialized from the NATS envelope payload field.
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InvoiceWrittenOffPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub written_off_amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub authorized_by: Option<String>,
    pub written_off_at: chrono::DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a write-off event and post the balanced GL journal entry.
///
/// Returns the created journal entry ID on success, or `JournalError` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
///
/// ## Journal entry
/// Direct write-off method:
///   DR  Bad Debt Expense ("BAD_DEBT") — expense recognized for uncollectable debt
///   CR  Accounts Receivable ("AR")    — removes the receivable from the books
pub async fn process_writeoff_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &InvoiceWrittenOffPayload,
) -> Result<Uuid, JournalError> {
    // Convert minor units to major units (cents → dollars)
    let amount = payload.written_off_amount_minor as f64 / 100.0;

    // Build balanced journal entry (direct write-off method):
    //   DR Bad Debt Expense ← recognizes the uncollectable amount as expense
    //   CR Accounts Receivable ← removes the receivable from the books
    let posting = GlPostingRequestV1 {
        posting_date: payload.written_off_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ArAdjustment,
        source_doc_id: payload.invoice_id.clone(),
        description: format!(
            "Write-off invoice {} — {}",
            payload.invoice_id, payload.reason
        ),
        lines: vec![
            JournalLine {
                account_ref: "BAD_DEBT".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Bad debt expense — write-off invoice {} ({})",
                    payload.invoice_id, payload.reason
                )),
                dimensions: Some(crate::contracts::gl_posting_request_v1::Dimensions {
                    customer_id: Some(payload.customer_id.clone()),
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: None,
                    project: None,
                }),
            },
            JournalLine {
                account_ref: "AR".to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "AR reduction — written off invoice {}",
                    payload.invoice_id
                )),
                dimensions: Some(crate::contracts::gl_posting_request_v1::Dimensions {
                    customer_id: Some(payload.customer_id.clone()),
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: None,
                    project: None,
                }),
            },
        ],
    };

    let subject = format!("ar.invoice_written_off.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None, // correlation_id propagation handled by envelope if needed
    )
    .await
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the GL write-off consumer task.
///
/// Subscribes to `ar.invoice_written_off` NATS subject and posts GL journal
/// entries via `process_writeoff_posting`.
pub async fn start_gl_writeoff_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL write-off consumer");

        let subject = "ar.invoice_written_off";
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
                "process_writeoff_gl_posting",
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
                            process_writeoff_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "gl_writeoff_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        "Write-off GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL write-off consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_writeoff_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<InvoiceWrittenOffPayload> =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse write-off envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        invoice_id = %envelope.payload.invoice_id,
        written_off_amount_minor = %envelope.payload.written_off_amount_minor,
        "Processing write-off GL posting"
    );

    match process_writeoff_posting(
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
                "Write-off GL journal entry created"
            );
            Ok(())
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            tracing::info!(event_id = %event_id, "Duplicate write-off event ignored");
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

//! GL Credit Note Posting Consumer (bd-3vm)
//!
//! Handles `ar.credit_note_issued` events and posts balanced journal entries to GL.
//!
//! ## Accounting Entry (credit note reduces AR and reverses revenue)
//!
//! ```text
//! DR  Revenue ("REV")         amount_minor / 100.0   ← revenue reduction
//! CR  Accounts Receivable ("AR")  amount_minor / 100.0   ← AR reduction
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

use crate::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use crate::services::journal_service::{process_gl_posting_request, JournalError};

// ============================================================================
// AR credit note payload (mirrors ar::events::contracts::CreditNoteIssuedPayload)
// Deserialized from the NATS envelope payload field.
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreditNoteIssuedPayload {
    pub credit_note_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub invoice_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub issued_at: chrono::DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a credit note event and post the balanced GL journal entry.
///
/// Returns the created journal entry ID on success, or `JournalError` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
pub async fn process_credit_note_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &CreditNoteIssuedPayload,
) -> Result<Uuid, JournalError> {
    // Convert minor units to major units (cents → dollars)
    let amount = payload.amount_minor as f64 / 100.0;

    // Build balanced journal entry:
    //   DR Revenue  ← reversal of recognized revenue
    //   CR AR       ← reduces receivable
    let posting = GlPostingRequestV1 {
        posting_date: payload.issued_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ArCreditMemo,
        source_doc_id: payload.credit_note_id.to_string(),
        description: format!(
            "Credit note {} against invoice {} — {}",
            payload.credit_note_id, payload.invoice_id, payload.reason
        ),
        lines: vec![
            JournalLine {
                account_ref: "REV".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Revenue reversal — credit note {}",
                    payload.credit_note_id
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
                    "AR reduction — credit note {}",
                    payload.credit_note_id
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

    let subject = format!("ar.credit_note_issued.{}", event_id);

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

/// Start the GL credit note consumer task.
///
/// Subscribes to `ar.credit_note_issued` NATS subject and posts GL journal
/// entries via `process_credit_note_posting`.
pub async fn start_gl_credit_note_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL credit note consumer");

        let subject = "ar.credit_note_issued";
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
                "process_credit_note_gl_posting",
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
                            process_credit_note_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "gl_credit_note_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        "Credit note GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL credit note consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_credit_note_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<CreditNoteIssuedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse credit note envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        credit_note_id = %envelope.payload.credit_note_id,
        amount_minor = %envelope.payload.amount_minor,
        "Processing credit note GL posting"
    );

    match process_credit_note_posting(
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
                "Credit note GL journal entry created"
            );
            Ok(())
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            tracing::info!(event_id = %event_id, "Duplicate credit note event ignored");
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

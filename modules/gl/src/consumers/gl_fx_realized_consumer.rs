//! GL Realized FX Gain/Loss Consumer (bd-1p8)
//!
//! Handles `ar.invoice_settled_fx` events and posts balanced journal entries to GL
//! for realized foreign exchange gains or losses on settlement.
//!
//! ## Accounting Entry
//!
//! When settlement rate > recognition rate (gain):
//! ```text
//! DR  Accounts Receivable ("AR")        delta   ← AR adjusted up to settlement rate
//! CR  FX Realized Gain ("FX_REALIZED_GAIN")  delta   ← gain recognized
//! ```
//!
//! When settlement rate < recognition rate (loss):
//! ```text
//! DR  FX Realized Loss ("FX_REALIZED_LOSS")  |delta| ← loss recognized
//! CR  Accounts Receivable ("AR")              |delta| ← AR adjusted down to settlement rate
//! ```
//!
//! When settlement rate == recognition rate (no FX difference):
//! No journal entry is posted — settlement produces no FX effect.
//!
//! ## Idempotency
//! Uses `processed_events` table via `process_gl_posting_request`. The event_id from
//! the incoming envelope is the idempotency key — duplicate events are silently skipped.

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
// AR FX settlement payload (mirrors ar::events::contracts::InvoiceSettledFxPayload)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InvoiceSettledFxPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub txn_currency: String,
    pub txn_amount_minor: i64,
    pub rpt_currency: String,
    pub recognition_rpt_amount_minor: i64,
    pub recognition_rate_id: Uuid,
    pub recognition_rate: f64,
    pub settlement_rpt_amount_minor: i64,
    pub settlement_rate_id: Uuid,
    pub settlement_rate: f64,
    pub realized_gain_loss_minor: i64,
    pub settled_at: chrono::DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process an FX settlement event and post the balanced GL journal entry.
///
/// Returns `Ok(Some(entry_id))` when a journal entry is created (rates differ),
/// `Ok(None)` when no entry is needed (rates match), or `Err(JournalError)` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
pub async fn process_fx_realized_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &InvoiceSettledFxPayload,
) -> Result<Option<Uuid>, JournalError> {
    let delta = payload.realized_gain_loss_minor;

    // No FX difference → no journal entry needed
    if delta == 0 {
        tracing::info!(
            event_id = %event_id,
            invoice_id = %payload.invoice_id,
            "No FX difference on settlement — skipping GL posting"
        );
        return Ok(None);
    }

    let abs_delta = delta.unsigned_abs() as f64 / 100.0;

    let (lines, description) = if delta > 0 {
        // FX Gain: DR AR, CR FX_REALIZED_GAIN
        (
            vec![
                JournalLine {
                    account_ref: "AR".to_string(),
                    debit: abs_delta,
                    credit: 0.0,
                    memo: Some(format!(
                        "AR FX adjustment — invoice {} settled at {} (recognized at {})",
                        payload.invoice_id, payload.settlement_rate, payload.recognition_rate
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
                    account_ref: "FX_REALIZED_GAIN".to_string(),
                    debit: 0.0,
                    credit: abs_delta,
                    memo: Some(format!(
                        "Realized FX gain — invoice {} ({} → {})",
                        payload.invoice_id, payload.txn_currency, payload.rpt_currency
                    )),
                    dimensions: None,
                },
            ],
            format!(
                "Realized FX gain on invoice {} — {} {} at rate {} vs {}",
                payload.invoice_id,
                payload.txn_currency,
                payload.txn_amount_minor as f64 / 100.0,
                payload.settlement_rate,
                payload.recognition_rate
            ),
        )
    } else {
        // FX Loss: DR FX_REALIZED_LOSS, CR AR
        (
            vec![
                JournalLine {
                    account_ref: "FX_REALIZED_LOSS".to_string(),
                    debit: abs_delta,
                    credit: 0.0,
                    memo: Some(format!(
                        "Realized FX loss — invoice {} ({} → {})",
                        payload.invoice_id, payload.txn_currency, payload.rpt_currency
                    )),
                    dimensions: None,
                },
                JournalLine {
                    account_ref: "AR".to_string(),
                    debit: 0.0,
                    credit: abs_delta,
                    memo: Some(format!(
                        "AR FX adjustment — invoice {} settled at {} (recognized at {})",
                        payload.invoice_id, payload.settlement_rate, payload.recognition_rate
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
            format!(
                "Realized FX loss on invoice {} — {} {} at rate {} vs {}",
                payload.invoice_id,
                payload.txn_currency,
                payload.txn_amount_minor as f64 / 100.0,
                payload.settlement_rate,
                payload.recognition_rate
            ),
        )
    };

    let posting = GlPostingRequestV1 {
        posting_date: payload.settled_at.format("%Y-%m-%d").to_string(),
        currency: payload.rpt_currency.to_uppercase(),
        source_doc_type: SourceDocType::ArAdjustment,
        source_doc_id: payload.invoice_id.clone(),
        description,
        lines,
    };

    let subject = format!("ar.invoice_settled_fx.{}", event_id);

    let entry_id = process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await?;

    Ok(Some(entry_id))
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the GL FX realized gain/loss consumer task.
///
/// Subscribes to `ar.invoice_settled_fx` NATS subject and posts GL journal
/// entries via `process_fx_realized_posting`.
pub async fn start_gl_fx_realized_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL FX realized gain/loss consumer");

        let subject = "ar.invoice_settled_fx";
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
                "process_fx_realized_gl_posting",
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
                            process_fx_realized_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "gl_fx_realized_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        "FX realized GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL FX realized consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_fx_realized_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<InvoiceSettledFxPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse FX settlement envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        invoice_id = %envelope.payload.invoice_id,
        realized_gain_loss_minor = %envelope.payload.realized_gain_loss_minor,
        "Processing FX realized GL posting"
    );

    match process_fx_realized_posting(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.source_module,
        &envelope.payload,
    )
    .await
    {
        Ok(Some(entry_id)) => {
            tracing::info!(
                event_id = %envelope.event_id,
                entry_id = %entry_id,
                "FX realized GL journal entry created"
            );
            Ok(())
        }
        Ok(None) => {
            tracing::info!(
                event_id = %envelope.event_id,
                "No FX difference — no GL entry needed"
            );
            Ok(())
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            tracing::info!(event_id = %event_id, "Duplicate FX settlement event ignored");
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

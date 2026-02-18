//! AR Tax Liability GL Consumer (bd-3gsz)
//!
//! Handles `tax.committed` and `tax.voided` events from the AR module and posts
//! balanced journal entries to GL for sales tax liability tracking.
//!
//! ## Accounting Entries
//!
//! ### tax.committed (tax liability recognized at invoice finalization)
//! ```text
//! DR  TAX_COLLECTED    total_tax_minor / 100.0   ← tax collected from customer (clearing)
//! CR  TAX_PAYABLE      total_tax_minor / 100.0   ← liability to tax authority
//! ```
//!
//! ### tax.voided (committed tax reversed on refund/write-off/cancellation)
//! ```text
//! DR  TAX_PAYABLE      total_tax_minor / 100.0   ← reduce liability
//! CR  TAX_COLLECTED    total_tax_minor / 100.0   ← reverse clearing balance
//! ```
//!
//! ## Design Rationale
//! Tax is NOT recomputed in GL. The `total_tax_minor` from AR's tax snapshot is
//! used directly. This ensures GL matches AR exactly and avoids rounding drift.
//!
//! ## Idempotency
//! Uses `processed_events` table via `process_gl_posting_request`. The event_id
//! from the incoming envelope is the idempotency key — duplicates silently skip.
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
    Dimensions, GlPostingRequestV1, JournalLine, SourceDocType,
};
use crate::services::journal_service::{process_gl_posting_request, JournalError};

/// Well-known GL account for tax collected from customers (clearing/offset)
pub const TAX_COLLECTED_ACCOUNT: &str = "TAX_COLLECTED";

/// Well-known GL account for sales tax payable to tax authorities (liability)
pub const TAX_PAYABLE_ACCOUNT: &str = "TAX_PAYABLE";

// ============================================================================
// Payload: tax.committed (mirrors ar::events::contracts::TaxCommittedPayload)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaxCommittedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub provider_quote_ref: String,
    pub provider_commit_ref: String,
    pub provider: String,
    pub committed_at: DateTime<Utc>,
}

// ============================================================================
// Payload: tax.voided (mirrors ar::events::contracts::TaxVoidedPayload)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaxVoidedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub provider_commit_ref: String,
    pub provider: String,
    pub void_reason: String,
    pub voided_at: DateTime<Utc>,
}

// ============================================================================
// Public processing functions (testable without NATS)
// ============================================================================

/// Process a tax.committed event and post the tax liability GL journal entry.
///
/// Returns the created journal entry ID on success, or `JournalError` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
pub async fn process_tax_committed_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &TaxCommittedPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_tax_minor as f64 / 100.0;

    let dims = Some(Dimensions {
        customer_id: Some(payload.customer_id.clone()),
        vendor_id: None,
        location_id: None,
        job_id: None,
        department: None,
        class: None,
        project: None,
    });

    let posting = GlPostingRequestV1 {
        posting_date: payload.committed_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: payload.invoice_id.clone(),
        description: format!(
            "Sales tax liability — invoice {} (provider: {})",
            payload.invoice_id, payload.provider
        ),
        lines: vec![
            JournalLine {
                account_ref: TAX_COLLECTED_ACCOUNT.to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Tax collected — invoice {} commit {}",
                    payload.invoice_id, payload.provider_commit_ref
                )),
                dimensions: dims.clone(),
            },
            JournalLine {
                account_ref: TAX_PAYABLE_ACCOUNT.to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Sales tax payable — invoice {} ({})",
                    payload.invoice_id, payload.provider
                )),
                dimensions: dims,
            },
        ],
    };

    let subject = format!("tax.committed.{}", event_id);

    process_gl_posting_request(pool, event_id, tenant_id, source_module, &subject, &posting, None)
        .await
}

/// Process a tax.voided event and post the reversal GL journal entry.
///
/// Returns the created journal entry ID on success, or `JournalError` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
pub async fn process_tax_voided_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &TaxVoidedPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_tax_minor as f64 / 100.0;

    let dims = Some(Dimensions {
        customer_id: Some(payload.customer_id.clone()),
        vendor_id: None,
        location_id: None,
        job_id: None,
        department: None,
        class: None,
        project: None,
    });

    let posting = GlPostingRequestV1 {
        posting_date: payload.voided_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ArAdjustment,
        source_doc_id: payload.invoice_id.clone(),
        description: format!(
            "Tax liability reversal — invoice {} ({})",
            payload.invoice_id, payload.void_reason
        ),
        lines: vec![
            JournalLine {
                account_ref: TAX_PAYABLE_ACCOUNT.to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Tax payable reversal — invoice {} ({})",
                    payload.invoice_id, payload.void_reason
                )),
                dimensions: dims.clone(),
            },
            JournalLine {
                account_ref: TAX_COLLECTED_ACCOUNT.to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Tax collected reversal — invoice {} void {}",
                    payload.invoice_id, payload.provider_commit_ref
                )),
                dimensions: dims,
            },
        ],
    };

    let subject = format!("tax.voided.{}", event_id);

    process_gl_posting_request(pool, event_id, tenant_id, source_module, &subject, &posting, None)
        .await
}

// ============================================================================
// NATS consumers (production entry points)
// ============================================================================

/// Start the GL tax committed consumer task.
///
/// Subscribes to `tax.committed` NATS subject and posts tax liability GL entries.
pub async fn start_ar_tax_committed_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting AR tax committed consumer");

        let subject = "tax.committed";
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
                        tracing::error!(subject = %msg.subject, error = %e, "Bad envelope");
                        continue;
                    }
                };

            let span = tracing::info_span!(
                "process_tax_committed_gl",
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
                            process_committed_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "ar_tax_committed_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(error = %error_msg, "Tax committed GL posting failed, DLQ");
                    crate::dlq::handle_processing_error(
                        &pool, &msg, &error_msg, retry_config.max_attempts as i32,
                    )
                    .await;
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("AR tax committed consumer stopped");
    });
}

/// Start the GL tax voided consumer task.
///
/// Subscribes to `tax.voided` NATS subject and posts reversal GL entries.
pub async fn start_ar_tax_voided_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting AR tax voided consumer");

        let subject = "tax.voided";
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
                        tracing::error!(subject = %msg.subject, error = %e, "Bad envelope");
                        continue;
                    }
                };

            let span = tracing::info_span!(
                "process_tax_voided_gl",
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
                            process_voided_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "ar_tax_voided_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(error = %error_msg, "Tax voided GL posting failed, DLQ");
                    crate::dlq::handle_processing_error(
                        &pool, &msg, &error_msg, retry_config.max_attempts as i32,
                    )
                    .await;
                }
            }
            .instrument(span)
            .await;
        }

        tracing::warn!("AR tax voided consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_committed_message(pool: &PgPool, msg: &BusMessage) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<TaxCommittedPayload> =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse tax.committed envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        invoice_id = %envelope.payload.invoice_id,
        total_tax_minor = %envelope.payload.total_tax_minor,
        "Processing tax committed GL posting"
    );

    match process_tax_committed_posting(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.source_module,
        &envelope.payload,
    )
    .await
    {
        Ok(entry_id) => {
            tracing::info!(event_id = %envelope.event_id, entry_id = %entry_id, "Tax liability GL entry created");
            Ok(())
        }
        Err(JournalError::DuplicateEvent(eid)) => {
            tracing::info!(event_id = %eid, "Duplicate tax.committed event ignored");
            Ok(())
        }
        Err(e) => Err(classify_journal_error(e)),
    }
}

async fn process_voided_message(pool: &PgPool, msg: &BusMessage) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<TaxVoidedPayload> =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse tax.voided envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        invoice_id = %envelope.payload.invoice_id,
        total_tax_minor = %envelope.payload.total_tax_minor,
        void_reason = %envelope.payload.void_reason,
        "Processing tax voided GL posting"
    );

    match process_tax_voided_posting(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.source_module,
        &envelope.payload,
    )
    .await
    {
        Ok(entry_id) => {
            tracing::info!(event_id = %envelope.event_id, entry_id = %entry_id, "Tax reversal GL entry created");
            Ok(())
        }
        Err(JournalError::DuplicateEvent(eid)) => {
            tracing::info!(event_id = %eid, "Duplicate tax.voided event ignored");
            Ok(())
        }
        Err(e) => Err(classify_journal_error(e)),
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

fn classify_journal_error(err: JournalError) -> ProcessingError {
    match err {
        JournalError::Validation(e) => ProcessingError::Validation(format!("Validation: {}", e)),
        JournalError::InvalidDate(e) => ProcessingError::Validation(format!("Invalid date: {}", e)),
        JournalError::Period(e) => ProcessingError::Validation(format!("Period error: {}", e)),
        JournalError::Balance(e) => ProcessingError::Retriable(format!("Balance error: {}", e)),
        JournalError::Database(e) => ProcessingError::Retriable(format!("Database error: {}", e)),
        JournalError::DuplicateEvent(_) => unreachable!("handled above"),
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

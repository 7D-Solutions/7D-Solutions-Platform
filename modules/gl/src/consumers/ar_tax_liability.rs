//! AR Tax Liability GL Consumer — NATS wiring for tax committed/voided events
//!
//! Subscribes to `tax.committed` and `tax.voided` events and delegates to
//! posting functions in `ar_tax_liability_posting`.

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::services::journal_service::JournalError;

// Re-export posting types and functions for backward compatibility
pub use super::ar_tax_liability_posting::{
    process_tax_committed_posting, process_tax_voided_posting, TaxCommittedPayload,
    TaxVoidedPayload, TAX_COLLECTED_ACCOUNT, TAX_PAYABLE_ACCOUNT,
};

// ============================================================================
// NATS consumers (production entry points)
// ============================================================================

/// Start the GL tax committed consumer task.
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

        tracing::warn!("AR tax committed consumer stopped");
    });
}

/// Start the GL tax voided consumer task.
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

        tracing::warn!("AR tax voided consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_committed_message(pool: &PgPool, msg: &BusMessage) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<TaxCommittedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| {
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

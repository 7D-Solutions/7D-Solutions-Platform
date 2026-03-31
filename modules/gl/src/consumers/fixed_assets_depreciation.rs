//! GL Fixed Assets Depreciation Consumer — NATS wiring
//!
//! Subscribes to `fa_depreciation_run.depreciation_run_completed` and delegates
//! to posting functions in `fixed_assets_depreciation_posting`.

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

use crate::services::journal_service::JournalError;

// Re-export posting types and functions for backward compatibility
pub use super::fixed_assets_depreciation_posting::{
    process_depreciation_entry_posting, DepreciationGlEntry, DepreciationRunCompletedPayload,
};

/// Start the GL fixed assets depreciation consumer task.
pub async fn start_fixed_assets_depreciation_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "fa_depreciation_run.depreciation_run_completed";
        tracing::info!(subject, "Starting GL fixed assets depreciation consumer");

        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "Failed to subscribe");
                return;
            }
        };

        tracing::info!(
            subject,
            "Subscribed to FA depreciation run completed events"
        );

        let retry_config = RetryConfig::default();

        while let Some(msg) = stream.next().await {
            let run_id = extract_run_id(&msg);
            let span = tracing::info_span!(
                "process_fa_depreciation_gl",
                run_id = %run_id.unwrap_or(Uuid::nil()),
            );

            async {
                let pool_clone = pool.clone();
                let msg_clone = msg.clone();

                let result = retry_with_backoff(
                    || {
                        let pool = pool_clone.clone();
                        let msg = msg_clone.clone();
                        async move {
                            process_depreciation_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "gl_fa_depreciation_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        "FA depreciation GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL fixed assets depreciation consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_depreciation_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let payload: DepreciationRunCompletedPayload =
        serde_json::from_slice(&msg.payload).map_err(|e| {
            ProcessingError::Validation(format!(
                "Failed to parse depreciation_run_completed payload: {}",
                e
            ))
        })?;

    tracing::info!(
        run_id = %payload.run_id,
        tenant_id = %payload.tenant_id,
        periods_posted = payload.periods_posted,
        gl_entries_count = payload.gl_entries.len(),
        "Processing FA depreciation GL posting run"
    );

    if payload.gl_entries.is_empty() {
        tracing::debug!(
            run_id = %payload.run_id,
            "No GL entries in depreciation run event — skipping"
        );
        return Ok(());
    }

    for entry in &payload.gl_entries {
        match process_depreciation_entry_posting(pool, &payload.tenant_id, entry).await {
            Ok(entry_id) => {
                tracing::info!(
                    run_id = %payload.run_id,
                    schedule_id = %entry.entry_id,
                    journal_entry_id = %entry_id,
                    "FA depreciation GL journal entry created"
                );
            }
            Err(JournalError::DuplicateEvent(eid)) => {
                tracing::info!(
                    run_id = %payload.run_id,
                    schedule_id = %eid,
                    "Duplicate FA depreciation entry ignored (idempotent)"
                );
            }
            Err(JournalError::Validation(e)) => {
                return Err(ProcessingError::Validation(format!("Validation: {}", e)));
            }
            Err(JournalError::InvalidDate(e)) => {
                return Err(ProcessingError::Validation(format!("Invalid date: {}", e)));
            }
            Err(JournalError::Period(e)) => {
                return Err(ProcessingError::Validation(format!("Period error: {}", e)));
            }
            Err(JournalError::Balance(e)) => {
                return Err(ProcessingError::Retriable(format!("Balance error: {}", e)));
            }
            Err(JournalError::Database(e)) => {
                return Err(ProcessingError::Retriable(format!("Database error: {}", e)));
            }
        }
    }

    Ok(())
}

fn extract_run_id(msg: &BusMessage) -> Option<Uuid> {
    let value: serde_json::Value = serde_json::from_slice(&msg.payload).ok()?;
    let s = value.get("run_id")?.as_str()?;
    Uuid::parse_str(s).ok()
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

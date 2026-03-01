//! GL Timekeeping Labor Cost Consumer
//!
//! Handles `timekeeping.labor_cost` events and posts balanced labor cost
//! accrual journal entries to GL.
//!
//! ## Accounting Entry
//!
//! ```text
//! DR  LABOR_EXPENSE   total_cost_minor / 100.0  ← labor cost recognized
//! CR  ACCRUED_LABOR   total_cost_minor / 100.0  ← accrued labor liability
//! ```
//!
//! ## Idempotency
//! Uses `processed_events` table via `process_gl_posting_request`. The deterministic
//! `posting_id` from the timekeeping module is the event_id / idempotency key.
//!
//! ## Period Validation
//! `process_gl_posting_request` enforces period existence and open/closed state.

use chrono::NaiveDate;
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

// ============================================================================
// Labor cost payload (mirrors timekeeping::domain::integrations::gl::service)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LaborCostPostingPayload {
    pub posting_id: Uuid,
    pub app_id: String,
    pub employee_id: Uuid,
    pub employee_name: String,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub total_minutes: i32,
    pub hourly_rate_minor: i64,
    pub currency: String,
    pub total_cost_minor: i64,
    pub posting_date: String,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a labor cost posting event and create the balanced GL journal entry.
///
/// Returns the created journal entry ID on success.
/// Duplicate posting_ids return `JournalError::DuplicateEvent` (idempotent no-op).
pub async fn process_labor_cost_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &LaborCostPostingPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_cost_minor as f64 / 100.0;
    let hours = payload.total_minutes as f64 / 60.0;

    let description = if let Some(ref proj) = payload.project_name {
        format!(
            "Labor cost accrual — {} ({:.1}h on {})",
            payload.employee_name, hours, proj,
        )
    } else {
        format!(
            "Labor cost accrual — {} ({:.1}h)",
            payload.employee_name, hours,
        )
    };

    let dimensions = Some(Dimensions {
        customer_id: None,
        vendor_id: None,
        location_id: None,
        job_id: payload.project_id.map(|p| p.to_string()),
        department: None,
        class: None,
        project: payload.project_name.clone(),
    });

    let posting = GlPostingRequestV1 {
        posting_date: payload.posting_date.clone(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::LaborCostAccrual,
        source_doc_id: payload.posting_id.to_string(),
        description,
        lines: vec![
            JournalLine {
                account_ref: "LABOR_EXPENSE".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Labor expense — {} ({:.1}h @ ${:.2}/hr)",
                    payload.employee_name,
                    hours,
                    payload.hourly_rate_minor as f64 / 100.0,
                )),
                dimensions: dimensions.clone(),
            },
            JournalLine {
                account_ref: "ACCRUED_LABOR".to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Accrued labor — {} ({:.1}h)",
                    payload.employee_name, hours,
                )),
                dimensions,
            },
        ],
    };

    let subject = format!("timekeeping.labor_cost.{}", event_id);

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

/// Start the GL timekeeping labor cost consumer task.
pub async fn start_gl_labor_cost_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL timekeeping labor cost consumer");

        let subject = "timekeeping.labor_cost";
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
                "process_labor_cost_posting",
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
                            process_labor_cost_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "gl_labor_cost_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        "Labor cost GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL timekeeping labor cost consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_labor_cost_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<LaborCostPostingPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| {
            ProcessingError::Validation(format!("Failed to parse labor_cost envelope: {}", e))
        })?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        employee_id = %envelope.payload.employee_id,
        total_cost_minor = %envelope.payload.total_cost_minor,
        "Processing timekeeping labor cost GL posting"
    );

    match process_labor_cost_posting(
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
                "Labor cost GL journal entry created"
            );
            Ok(())
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            tracing::info!(event_id = %event_id, "Duplicate labor_cost event ignored");
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

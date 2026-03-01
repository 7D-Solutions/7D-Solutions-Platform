//! GL AP Vendor Bill Approved Consumer (bd-1l86)
//!
//! Handles `ap.vendor_bill_approved` events and posts balanced AP liability +
//! expense journal entries to GL.
//!
//! ## Accounting Entry
//!
//! ```text
//! For each bill line (from gl_lines in the event payload):
//!   DR  <gl_account_code>  line.amount  ← expense (or AP_CLEARING if PO-backed)
//! CR  AP                 total_amount  ← accounts payable liability
//! ```
//!
//! When `po_line_id` is Some on a line, the debit posts to `AP_CLEARING`
//! (inventory clearing account) rather than the expense account.
//!
//! When `gl_lines` is empty (fallback), the full approved amount is debited
//! to `EXPENSE`.
//!
//! ## FX / Multi-Currency
//!
//! The event carries `fx_rate_id` (Phase 23a identifier). When present:
//! - Rate looked up from `fx_rates` table by UUID.
//! - Amounts converted to reporting currency via `currency_conversion::convert_journal_lines`.
//! - Journal posted in reporting currency; original currency noted in description.
//!
//! When `fx_rate_id` is None (same-currency bill), amounts posted as-is.
//!
//! ## Idempotency
//!
//! Uses `processed_events` table via `process_gl_posting_request`. The event_id
//! from the incoming envelope is the idempotency key — duplicate events are
//! silently skipped.
//!
//! ## Period Validation
//!
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
use crate::services::currency_conversion::{
    convert_journal_lines, requires_conversion, RateSnapshot,
};
use crate::services::journal_service::{process_gl_posting_request, JournalError};

/// AP clearing account used for PO-backed bill lines (inventory receipt linkage).
pub const AP_CLEARING_ACCOUNT: &str = "AP_CLEARING";
/// Default AP liability account for the credit side.
pub const AP_LIABILITY_ACCOUNT: &str = "AP";
/// Fallback expense account when `gl_lines` is empty.
pub const DEFAULT_EXPENSE_ACCOUNT: &str = "EXPENSE";

// ============================================================================
// Local mirror of AP event payload
// Deserialized from the NATS EventEnvelope payload field.
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApprovedGlLine {
    pub line_id: Uuid,
    pub gl_account_code: String,
    pub amount_minor: i64,
    pub po_line_id: Option<Uuid>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct VendorBillApprovedPayload {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub vendor_invoice_ref: String,
    pub approved_amount_minor: i64,
    pub currency: String,
    pub due_date: DateTime<Utc>,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
    /// Phase 23a FX infrastructure identifier. None when same-currency bill.
    pub fx_rate_id: Option<Uuid>,
    /// Per-line GL account allocations. Empty = fallback to DEFAULT_EXPENSE_ACCOUNT.
    pub gl_lines: Vec<ApprovedGlLine>,
}

// ============================================================================
// FX rate row (queried from GL fx_rates table by UUID)
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
struct FxRateRow {
    pub id: Uuid,
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub inverse_rate: f64,
    pub effective_at: DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a vendor_bill_approved event and post the balanced AP GL journal entry.
///
/// Returns the created journal entry ID on success, or `JournalError` on failure.
/// Duplicate event_ids return `JournalError::DuplicateEvent` (idempotent no-op).
///
/// ## Journal entry
///   DR expense accounts (per gl_lines) or EXPENSE (fallback)
///   CR AP — accounts payable liability
pub async fn process_ap_bill_approved_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &VendorBillApprovedPayload,
) -> Result<Uuid, JournalError> {
    let posting_date = payload.approved_at.format("%Y-%m-%d").to_string();
    let vendor_id_str = payload.vendor_id.to_string();

    // Resolve per-line debit allocations: (account_ref, amount_minor)
    let debit_lines: Vec<(String, i64)> = if payload.gl_lines.is_empty() {
        vec![(
            DEFAULT_EXPENSE_ACCOUNT.to_string(),
            payload.approved_amount_minor,
        )]
    } else {
        payload
            .gl_lines
            .iter()
            .map(|l| {
                let account = if l.po_line_id.is_some() {
                    AP_CLEARING_ACCOUNT.to_string()
                } else {
                    l.gl_account_code.clone()
                };
                (account, l.amount_minor)
            })
            .collect()
    };

    let total_debits_minor: i64 = debit_lines.iter().map(|(_, amt)| amt).sum();

    // Build balanced line pairs: (debit_minor, credit_minor) for optional FX conversion
    let mut line_pairs: Vec<(i64, i64)> = debit_lines.iter().map(|(_, amt)| (*amt, 0i64)).collect();
    line_pairs.push((0i64, total_debits_minor));

    // Determine posting currency and optionally convert via FX rate
    let (posting_currency, converted_pairs) =
        resolve_posting_currency(pool, tenant_id, payload, line_pairs).await?;

    // Build GlPostingRequestV1 journal lines
    let mut journal_lines: Vec<JournalLine> = Vec::with_capacity(debit_lines.len() + 1);

    for (i, (account, _)) in debit_lines.iter().enumerate() {
        let (debit_minor, _) = converted_pairs[i];
        journal_lines.push(JournalLine {
            account_ref: account.clone(),
            debit: debit_minor as f64 / 100.0,
            credit: 0.0,
            memo: Some(format!(
                "AP expense — bill {} ({})",
                payload.vendor_invoice_ref, payload.currency
            )),
            dimensions: Some(Dimensions {
                customer_id: None,
                vendor_id: Some(vendor_id_str.clone()),
                location_id: None,
                job_id: None,
                department: None,
                class: None,
                project: None,
            }),
        });
    }

    let (_, credit_minor) = converted_pairs[debit_lines.len()];
    journal_lines.push(JournalLine {
        account_ref: AP_LIABILITY_ACCOUNT.to_string(),
        debit: 0.0,
        credit: credit_minor as f64 / 100.0,
        memo: Some(format!(
            "AP liability — bill {} vendor {} ({})",
            payload.vendor_invoice_ref, payload.vendor_id, payload.currency
        )),
        dimensions: Some(Dimensions {
            customer_id: None,
            vendor_id: Some(vendor_id_str.clone()),
            location_id: None,
            job_id: None,
            department: None,
            class: None,
            project: None,
        }),
    });

    let posting = GlPostingRequestV1 {
        posting_date,
        currency: posting_currency.to_uppercase(),
        source_doc_type: SourceDocType::ApBill,
        source_doc_id: payload.bill_id.to_string(),
        description: format!(
            "AP bill approved — {} / vendor {} ({}{})",
            payload.vendor_invoice_ref,
            payload.vendor_id,
            payload.currency,
            payload
                .fx_rate_id
                .map(|id| format!(" FX:{}", id))
                .unwrap_or_default(),
        ),
        lines: journal_lines,
    };

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &format!("ap.vendor_bill_approved.{}", event_id),
        &posting,
        None,
    )
    .await
}

/// Resolve the posting currency and convert amounts when `fx_rate_id` is present.
///
/// Returns `(posting_currency, converted_line_pairs)`.
async fn resolve_posting_currency(
    pool: &PgPool,
    tenant_id: &str,
    payload: &VendorBillApprovedPayload,
    line_pairs: Vec<(i64, i64)>,
) -> Result<(String, Vec<(i64, i64)>), JournalError> {
    let Some(rate_id) = payload.fx_rate_id else {
        return Ok((payload.currency.clone(), line_pairs));
    };

    let rate_row: Option<FxRateRow> = sqlx::query_as(
        r#"
        SELECT id, base_currency, quote_currency, rate, inverse_rate, effective_at
        FROM fx_rates
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(rate_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(JournalError::Database)?;

    let Some(rate) = rate_row else {
        tracing::warn!(
            bill_id = %payload.bill_id,
            fx_rate_id = %rate_id,
            "FX rate not found in GL; posting in original bill currency"
        );
        return Ok((payload.currency.clone(), line_pairs));
    };

    let bill_ccy = payload.currency.to_uppercase();
    let reporting_currency = if rate.base_currency.to_uppercase() != bill_ccy {
        rate.base_currency.clone()
    } else {
        rate.quote_currency.clone()
    };

    if !requires_conversion(&bill_ccy, &reporting_currency) {
        return Ok((payload.currency.clone(), line_pairs));
    }

    let snapshot = RateSnapshot {
        rate_id: rate.id,
        rate: rate.rate,
        inverse_rate: rate.inverse_rate,
        effective_at: rate.effective_at,
        base_currency: rate.base_currency.clone(),
        quote_currency: rate.quote_currency.clone(),
    };

    let converted =
        convert_journal_lines(&line_pairs, &snapshot, &bill_ccy, &reporting_currency)
            .map_err(|e| JournalError::InvalidDate(format!("FX conversion failed: {}", e)))?;

    let pairs: Vec<(i64, i64)> = converted
        .iter()
        .map(|l| (l.rpt_debit_minor, l.rpt_credit_minor))
        .collect();

    Ok((reporting_currency, pairs))
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the GL AP vendor bill approved consumer task.
///
/// Subscribes to `ap.vendor_bill_approved` NATS subject and posts AP liability
/// + expense GL journal entries via `process_ap_bill_approved_posting`.
pub async fn start_ap_vendor_bill_approved_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("Starting GL AP vendor bill approved consumer");

        let subject = "ap.vendor_bill_approved";
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
                "process_ap_bill_approved_gl",
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
                            process_ap_bill_message(&pool, &msg)
                                .await
                                .map_err(format_error_for_retry)
                        }
                    },
                    &retry_config,
                    "gl_ap_bill_approved_consumer",
                )
                .await;

                if let Err(error_msg) = result {
                    tracing::error!(
                        error = %error_msg,
                        "AP bill approved GL posting failed after retries, sending to DLQ"
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

        tracing::warn!("GL AP vendor bill approved consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_ap_bill_message(pool: &PgPool, msg: &BusMessage) -> Result<(), ProcessingError> {
    let envelope: EventEnvelope<VendorBillApprovedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| {
        ProcessingError::Validation(format!(
            "Failed to parse vendor_bill_approved envelope: {}",
            e
        ))
    })?;

    tracing::info!(
        event_id = %envelope.event_id,
        bill_id = %envelope.payload.bill_id,
        currency = %envelope.payload.currency,
        fx_rate_id = ?envelope.payload.fx_rate_id,
        gl_lines_count = %envelope.payload.gl_lines.len(),
        "Processing AP bill approved GL posting"
    );

    match process_ap_bill_approved_posting(
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
                "AP bill approved GL journal entry created"
            );
            Ok(())
        }
        Err(JournalError::DuplicateEvent(event_id)) => {
            tracing::info!(event_id = %event_id, "Duplicate vendor_bill_approved event ignored");
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

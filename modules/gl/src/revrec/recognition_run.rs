//! Recognition Run Engine — Phase 24a (bd-b9m)
//!
//! Periodic job that selects due schedule rows for a target period, marks them
//! recognized, and posts balanced GL journal entries.
//!
//! ## Idempotency
//! - Each schedule line can only be recognized once (enforced by `recognized` flag).
//! - The run_id is deterministically derived from (schedule_id, period) via UUID v5,
//!   so re-running for the same period produces the same identifiers.
//! - Outbox event_ids are also deterministic per (line_id, period).
//!
//! ## Journal semantics
//! Each recognized line produces a balanced two-line journal entry:
//!   DR  Deferred Revenue  (reduces liability)
//!   CR  Revenue           (recognizes income)
//!
//! ## Scope
//! Only considers lines from the **latest** schedule version for each obligation,
//! preventing double-recognition when schedules are re-versioned after amendments.

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::repos::journal_repo::{self, JournalLineInsert};
use crate::repos::outbox_repo;
use crate::repos::revrec_repo::{self, DueScheduleLine};
use crate::revrec::{
    RecognitionPostedPayload, EVENT_TYPE_RECOGNITION_POSTED, MUTATION_CLASS_DATA_MUTATION,
};

/// UUID v5 namespace for recognition run IDs (deterministic from schedule_id + period)
const RECOGNITION_RUN_NS: Uuid = Uuid::from_bytes([
    0x72, 0x65, 0x76, 0x72, 0x65, 0x63, 0x2d, 0x72, 0x75, 0x6e, 0x2d, 0x6e, 0x73, 0x2d, 0x76, 0x31,
]);

/// Errors from recognition run execution
#[derive(Debug, thiserror::Error)]
pub enum RecognitionRunError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Invalid posting_date: {0}")]
    InvalidPostingDate(String),

    #[error("Repo error: {0}")]
    Repo(#[from] revrec_repo::RevrecRepoError),
}

/// Summary of a completed recognition run
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecognitionRunResult {
    /// Target period (YYYY-MM)
    pub period: String,
    /// Tenant that was processed
    pub tenant_id: String,
    /// Number of schedule lines recognized in this run
    pub lines_recognized: usize,
    /// Number of lines that were already recognized (skipped)
    pub lines_skipped: usize,
    /// Total amount recognized in this run (minor currency units)
    pub total_recognized_minor: i64,
    /// Individual posting details
    pub postings: Vec<RecognitionPosting>,
}

/// Details of a single recognition posting
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecognitionPosting {
    pub run_id: Uuid,
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub journal_entry_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
}

/// Execute a recognition run for a tenant and period.
///
/// Selects all unrecognized schedule lines due for the period (from latest
/// schedule versions only), posts balanced journal entries, marks lines
/// recognized, and emits outbox events — all atomically per line.
///
/// Idempotent: re-running for the same period skips already-recognized lines.
pub async fn run_recognition(
    pool: &PgPool,
    tenant_id: &str,
    period: &str,
    posting_date: &str,
) -> Result<RecognitionRunResult, RecognitionRunError> {
    // Validate posting_date
    let posting_date_parsed = NaiveDate::parse_from_str(posting_date, "%Y-%m-%d")
        .map_err(|e| RecognitionRunError::InvalidPostingDate(format!("{}: {}", posting_date, e)))?;
    let posted_at = posting_date_parsed
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| RecognitionRunError::InvalidPostingDate("Invalid time".to_string()))?
        .and_utc();

    // Find all due (unrecognized) lines for this period
    let due_lines = revrec_repo::find_due_lines_for_period(pool, tenant_id, period).await?;

    tracing::info!(
        tenant_id = %tenant_id,
        period = %period,
        due_lines = due_lines.len(),
        "Recognition run: found due schedule lines"
    );

    let mut result = RecognitionRunResult {
        period: period.to_string(),
        tenant_id: tenant_id.to_string(),
        lines_recognized: 0,
        lines_skipped: 0,
        total_recognized_minor: 0,
        postings: Vec::new(),
    };

    // Process each due line in its own transaction for atomicity
    for line in &due_lines {
        match recognize_single_line(pool, line, period, posting_date, &posted_at).await {
            Ok(Some(posting)) => {
                result.total_recognized_minor += posting.amount_minor;
                result.postings.push(posting);
                result.lines_recognized += 1;
            }
            Ok(None) => {
                // Line was already recognized (race condition / idempotency)
                result.lines_skipped += 1;
            }
            Err(e) => {
                tracing::error!(
                    line_id = line.line_id,
                    schedule_id = %line.schedule_id,
                    error = %e,
                    "Failed to recognize line, aborting run"
                );
                return Err(e);
            }
        }
    }

    tracing::info!(
        tenant_id = %tenant_id,
        period = %period,
        recognized = result.lines_recognized,
        skipped = result.lines_skipped,
        total_minor = result.total_recognized_minor,
        "Recognition run complete"
    );

    Ok(result)
}

/// Recognize a single schedule line: journal + mark + outbox, all atomic.
///
/// Returns `Ok(Some(posting))` if the line was newly recognized,
/// `Ok(None)` if it was already recognized (idempotent skip).
async fn recognize_single_line(
    pool: &PgPool,
    line: &DueScheduleLine,
    period: &str,
    posting_date: &str,
    posted_at: &chrono::DateTime<Utc>,
) -> Result<Option<RecognitionPosting>, RecognitionRunError> {
    // Deterministic run_id from (schedule_id, period)
    let run_id = Uuid::new_v5(
        &RECOGNITION_RUN_NS,
        format!("{}:{}", line.schedule_id, period).as_bytes(),
    );

    // Deterministic event_id from (line_id, period) for outbox dedup
    let event_id = Uuid::new_v5(
        &RECOGNITION_RUN_NS,
        format!("event:{}:{}", line.line_id, period).as_bytes(),
    );

    let journal_entry_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    // Mark the line recognized (returns 0 if already done)
    let rows_affected = revrec_repo::mark_line_recognized(&mut tx, line.line_id).await?;
    if rows_affected == 0 {
        tx.rollback().await?;
        return Ok(None);
    }

    // Create balanced journal entry: DR deferred, CR revenue
    let description = format!(
        "Revenue recognition: {} period {} (schedule {})",
        line.obligation_id, period, line.schedule_id
    );

    journal_repo::insert_entry(
        &mut tx,
        journal_entry_id,
        &line.tenant_id,
        "gl-revrec",
        event_id,
        "revrec.recognition_run",
        *posted_at,
        &line.currency,
        Some(&description),
        Some("REVREC_RECOGNITION"),
        Some(&line.schedule_id.to_string()),
        None, // correlation_id
    )
    .await?;

    let journal_lines = vec![
        JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: 1,
            account_ref: line.deferred_revenue_account.clone(),
            debit_minor: line.amount_to_recognize_minor,
            credit_minor: 0,
            memo: Some(format!("DR deferred revenue — period {}", period)),
        },
        JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: 2,
            account_ref: line.recognized_revenue_account.clone(),
            debit_minor: 0,
            credit_minor: line.amount_to_recognize_minor,
            memo: Some(format!("CR revenue recognized — period {}", period)),
        },
    ];

    journal_repo::bulk_insert_lines(&mut tx, journal_entry_id, &journal_lines).await?;

    // Compute cumulative amounts for the event payload.
    // We need to count this line's amount since it's just now being recognized.
    let previously_recognized: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(amount_to_recognize_minor), 0)::BIGINT
        FROM revrec_schedule_lines
        WHERE schedule_id = $1
          AND recognized = true
          AND id != $2
          AND period <= $3
        "#,
    )
    .bind(line.schedule_id)
    .bind(line.line_id)
    .bind(period)
    .fetch_one(&mut *tx)
    .await?;

    let cumulative = previously_recognized.unwrap_or(0) + line.amount_to_recognize_minor;

    // Get total for schedule to compute remaining
    let total: i64 = sqlx::query_scalar(
        "SELECT total_to_recognize_minor FROM revrec_schedules WHERE schedule_id = $1",
    )
    .bind(line.schedule_id)
    .fetch_one(&mut *tx)
    .await?;

    let remaining = total - cumulative;

    // Build recognition posted payload
    let payload = RecognitionPostedPayload {
        run_id,
        contract_id: line.contract_id,
        obligation_id: line.obligation_id,
        schedule_id: line.schedule_id,
        tenant_id: line.tenant_id.clone(),
        period: period.to_string(),
        posting_date: posting_date.to_string(),
        amount_recognized_minor: line.amount_to_recognize_minor,
        currency: line.currency.clone(),
        deferred_revenue_account: line.deferred_revenue_account.clone(),
        recognized_revenue_account: line.recognized_revenue_account.clone(),
        journal_entry_id: Some(journal_entry_id),
        cumulative_recognized_minor: cumulative,
        remaining_deferred_minor: remaining,
        recognized_at: Utc::now(),
    };

    // Insert outbox event atomically
    let outbox_payload =
        serde_json::to_value(&payload).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_RECOGNITION_POSTED,
        "revrec_recognition",
        &run_id.to_string(),
        outbox_payload,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        run_id = %run_id,
        schedule_id = %line.schedule_id,
        period = %period,
        amount = line.amount_to_recognize_minor,
        journal_entry_id = %journal_entry_id,
        cumulative = cumulative,
        remaining = remaining,
        "Recognition posted"
    );

    Ok(Some(RecognitionPosting {
        run_id,
        schedule_id: line.schedule_id,
        contract_id: line.contract_id,
        obligation_id: line.obligation_id,
        journal_entry_id,
        amount_minor: line.amount_to_recognize_minor,
        currency: line.currency.clone(),
    }))
}

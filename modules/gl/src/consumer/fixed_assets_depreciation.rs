//! GL Fixed Assets Depreciation Consumer
//!
//! Handles `fa_depreciation_run.depreciation_run_completed` events and posts
//! balanced depreciation journal entries to GL — one entry per schedule period.
//!
//! ## Accounting Entry (per depreciation schedule period)
//!
//! ```text
//! DR  <expense_account_ref>      amount   ← depreciation expense recognized
//! CR  <accum_depreciation_ref>   amount   ← accumulated depreciation built up
//! ```
//!
//! Account refs come from `fa_categories` (e.g. "6100" / "1510") and are
//! embedded in the event payload by the Fixed Assets module.
//!
//! ## Idempotency
//!
//! Each depreciation schedule row has a unique `entry_id` (UUID). The GL
//! consumer uses `entry_id` as the `process_gl_posting_request` event_id.
//! On replay, `processed_events` detects the duplicate and returns
//! `JournalError::DuplicateEvent` — silently skipped.
//!
//! ## Period Validation
//!
//! `process_gl_posting_request` enforces period existence and open/closed state.
//! Events targeting a closed period are non-retriable → sent to DLQ.

use chrono::NaiveDate;
use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{BusMessage, EventBus};
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
// Event payload mirror (anti-corruption layer)
// Mirrors fixed_assets::domain::depreciation::models::DepreciationRunCompletedEvent
// ============================================================================

/// Per-entry GL data embedded in the run-completed event.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DepreciationGlEntry {
    /// fa_depreciation_schedules.id — idempotency key for GL posting.
    pub entry_id: Uuid,
    pub asset_id: Uuid,
    pub period_end: NaiveDate,
    pub depreciation_amount_minor: i64,
    pub currency: String,
    /// fa_categories.depreciation_expense_ref (e.g. "6100")
    pub expense_account_ref: String,
    /// fa_categories.accum_depreciation_ref (e.g. "1510")
    pub accum_depreciation_ref: String,
}

/// Mirror of fixed_assets::domain::depreciation::models::DepreciationRunCompletedEvent.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DepreciationRunCompletedPayload {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub periods_posted: i32,
    pub total_depreciation_minor: i64,
    /// Per-schedule GL posting data. Empty → no-op (nothing to post).
    #[serde(default)]
    pub gl_entries: Vec<DepreciationGlEntry>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Post a balanced GL depreciation journal entry for one schedule period.
///
/// ## Journal entry
///   DR  expense_account_ref  (depreciation expense)
///   CR  accum_depreciation_ref (accumulated depreciation)
///
/// Returns the created journal entry ID, or `JournalError::DuplicateEvent` when
/// the entry was already posted (idempotent replay).
pub async fn process_depreciation_entry_posting(
    pool: &PgPool,
    tenant_id: &str,
    entry: &DepreciationGlEntry,
) -> Result<Uuid, JournalError> {
    let amount = entry.depreciation_amount_minor as f64 / 100.0;
    let posting_date = entry.period_end.to_string();
    let asset_id_str = entry.asset_id.to_string();

    let posting = GlPostingRequestV1 {
        posting_date: posting_date.clone(),
        currency: entry.currency.to_uppercase(),
        source_doc_type: SourceDocType::FixedAssetDepreciation,
        source_doc_id: entry.entry_id.to_string(),
        description: format!(
            "Depreciation — asset {} period ending {}",
            entry.asset_id, entry.period_end,
        ),
        lines: vec![
            JournalLine {
                account_ref: entry.expense_account_ref.clone(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Depreciation expense — asset {} ({})",
                    entry.asset_id, entry.currency.to_uppercase(),
                )),
                dimensions: Some(Dimensions {
                    customer_id: None,
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: Some("fixed_assets".to_string()),
                    project: None,
                }),
            },
            JournalLine {
                account_ref: entry.accum_depreciation_ref.clone(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Accumulated depreciation — asset {} period ending {}",
                    entry.asset_id, entry.period_end,
                )),
                dimensions: Some(Dimensions {
                    customer_id: None,
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: Some("fixed_assets".to_string()),
                    project: Some(asset_id_str),
                }),
            },
        ],
    };

    let subject = format!("fa.depreciation.entry.{}", entry.entry_id);

    process_gl_posting_request(pool, entry.entry_id, tenant_id, "fixed-assets", &subject, &posting, None)
        .await
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the GL fixed assets depreciation consumer task.
///
/// Subscribes to `fa_depreciation_run.depreciation_run_completed` and posts
/// balanced depreciation journal entries for each schedule period in the run.
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

        tracing::info!(subject, "Subscribed to FA depreciation run completed events");

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

// ============================================================================
// Integrated tests — require running GL Postgres instance
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-fa-depr-gl-consumer";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to GL test DB")
    }

    /// Insert an open accounting period covering 2026-01 for TEST_TENANT.
    async fn ensure_open_period(pool: &PgPool) {
        sqlx::query(
            r#"
            INSERT INTO accounting_periods
                (id, tenant_id, period_start, period_end,
                 is_closed, created_at)
            VALUES (gen_random_uuid(), $1, '2026-01-01', '2026-01-31', FALSE, NOW())
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    }

    /// Insert GL expense (6100) and contra-asset (1510) accounts for TEST_TENANT.
    async fn ensure_accounts(pool: &PgPool) {
        for (code, name, atype, nb) in [
            ("6100", "Depreciation Expense", "expense", "debit"),
            ("1510", "Accumulated Depreciation", "asset", "credit"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO accounts
                    (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
                VALUES (gen_random_uuid(), $1, $2, $3, $4::account_type, $5::normal_balance, TRUE, NOW())
                ON CONFLICT (tenant_id, code) DO NOTHING
                "#,
            )
            .bind(TEST_TENANT)
            .bind(code)
            .bind(name)
            .bind(atype)
            .bind(nb)
            .execute(pool)
            .await
            .ok();
        }
    }

    async fn cleanup(pool: &PgPool) {
        for q in [
            "DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
            "DELETE FROM journal_lines     WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
            "DELETE FROM journal_entries   WHERE tenant_id = $1",
            "DELETE FROM account_balances  WHERE tenant_id = $1",
            "DELETE FROM accounting_periods WHERE tenant_id = $1",
            "DELETE FROM accounts           WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
        }
    }

    fn make_entry(amount: i64) -> DepreciationGlEntry {
        DepreciationGlEntry {
            entry_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            period_end: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            depreciation_amount_minor: amount,
            currency: "USD".to_string(),
            expense_account_ref: "6100".to_string(),
            accum_depreciation_ref: "1510".to_string(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn posts_balanced_journal_entry() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        ensure_open_period(&pool).await;
        ensure_accounts(&pool).await;

        let entry = make_entry(10_000);

        process_depreciation_entry_posting(&pool, TEST_TENANT, &entry)
            .await
            .expect("posting should succeed");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("count journal entries");
        assert_eq!(count, 1, "exactly one journal entry created");

        let lines: Vec<(String, f64, f64)> = sqlx::query_as(
            "SELECT account_ref, debit_minor::float8/100.0, credit_minor::float8/100.0 FROM journal_lines \
             WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .fetch_all(&pool)
        .await
        .expect("fetch lines");
        assert_eq!(lines.len(), 2, "two lines (DR + CR)");

        let debit_line = lines.iter().find(|(a, d, _)| a == "6100" && *d > 0.0).expect("DR line");
        let credit_line = lines.iter().find(|(a, _, c)| a == "1510" && *c > 0.0).expect("CR line");
        assert_eq!(debit_line.1, credit_line.2, "balanced: debit == credit");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn idempotent_on_replay() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        ensure_open_period(&pool).await;
        ensure_accounts(&pool).await;

        let entry = make_entry(5_000);

        process_depreciation_entry_posting(&pool, TEST_TENANT, &entry)
            .await
            .expect("first posting");

        // Replay: must return DuplicateEvent, not create a second entry
        let result = process_depreciation_entry_posting(&pool, TEST_TENANT, &entry).await;
        assert!(
            matches!(result, Err(JournalError::DuplicateEvent(_))),
            "replay must return DuplicateEvent, got {:?}", result
        );

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(count, 1, "no duplicate journal entries on replay");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn multiple_entries_in_run_all_posted() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        ensure_open_period(&pool).await;
        ensure_accounts(&pool).await;

        let entries = vec![make_entry(10_000), make_entry(10_000), make_entry(10_000)];

        for entry in &entries {
            process_depreciation_entry_posting(&pool, TEST_TENANT, entry)
                .await
                .expect("posting should succeed");
        }

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(count, 3, "one journal entry per schedule period");

        cleanup(&pool).await;
    }
}

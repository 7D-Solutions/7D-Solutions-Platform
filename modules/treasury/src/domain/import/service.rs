//! Statement import service — hashes CSV, creates statement + transaction lines.
//!
//! Idempotency is two-layer:
//! 1. `statement_hash` (UUID v5 of raw CSV bytes) on the statement row — re-import
//!    of the same file short-circuits with the existing statement ID.
//! 2. `external_id` on each transaction line — `ON CONFLICT DO NOTHING` prevents
//!    duplicate rows even if the hash check is somehow bypassed.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::adapters::CsvFormat;
use super::repo;
use super::{parser, ImportError, ImportResult, LineError};
use crate::domain::accounts::AccountStatus;
use crate::outbox::enqueue_event_tx;

/// UUID v5 namespace for statement content hashing.
const STATEMENT_HASH_NS: Uuid = Uuid::from_bytes([
    0x7d, 0x50, 0x1a, 0x71, 0xba, 0x4c, 0x4e, 0x2a, 0x8f, 0x01, 0xc3, 0xee, 0xd4, 0xa1, 0xb7, 0x09,
]);

const EVT_STATEMENT_IMPORTED: &str = "bank_statement.imported";

// ============================================================================
// Public request type
// ============================================================================

pub struct ImportRequest {
    pub account_id: Uuid,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub opening_balance_minor: i64,
    pub closing_balance_minor: i64,
    pub csv_data: Vec<u8>,
    pub filename: Option<String>,
    /// Optional CSV format hint. When `None`, the parser auto-detects
    /// from the CSV headers, falling back to the generic bank format.
    pub format: Option<CsvFormat>,
}

// ============================================================================
// Import entry point
// ============================================================================

pub async fn import_statement(
    pool: &PgPool,
    app_id: &str,
    req: ImportRequest,
    correlation_id: String,
) -> Result<ImportResult, ImportError> {
    // 1. Compute content hash
    let statement_hash = Uuid::new_v5(&STATEMENT_HASH_NS, &req.csv_data);

    // 2. Verify account exists and is active
    verify_account(pool, app_id, req.account_id).await?;

    // 3. Check for duplicate import (same file re-uploaded)
    if let Some(existing_id) =
        repo::find_statement_by_hash(pool, app_id, req.account_id, statement_hash).await?
    {
        return Err(ImportError::DuplicateImport {
            statement_id: existing_id,
        });
    }

    // 4. Parse CSV (auto-detects format if not specified)
    let parsed = parser::parse_csv_with_format(&req.csv_data, req.format);
    if parsed.lines.is_empty() {
        if parsed.errors.is_empty() {
            return Err(ImportError::EmptyImport);
        }
        return Err(ImportError::AllLinesFailed(parsed.errors));
    }

    // 5. Validate period
    if req.period_start > req.period_end {
        return Err(ImportError::Validation(
            "period_start must be <= period_end".to_string(),
        ));
    }

    // 6. Transactional: create statement + insert lines + emit event
    let statement_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = repo::fetch_account_currency(pool, app_id, req.account_id).await?;

    let mut tx = pool.begin().await?;

    repo::insert_statement_header(
        &mut tx,
        statement_id,
        app_id,
        req.account_id,
        req.period_start,
        req.period_end,
        req.opening_balance_minor,
        req.closing_balance_minor,
        &currency,
        req.filename.as_deref(),
        statement_hash,
        now,
    )
    .await?;

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let line_errors: Vec<LineError> = parsed.errors;

    for (idx, line) in parsed.lines.iter().enumerate() {
        let ext_id = format!("stmt:{}:line:{}", statement_hash, idx);
        let inserted = repo::insert_txn_line(
            &mut tx,
            app_id,
            req.account_id,
            statement_id,
            line.date,
            line.amount_minor,
            &currency,
            &line.description,
            line.reference.as_deref(),
            &ext_id,
        )
        .await?;
        if inserted {
            imported += 1;
        } else {
            skipped += 1;
        }
    }

    let payload = serde_json::json!({
        "statement_id": statement_id,
        "account_id": req.account_id,
        "app_id": app_id,
        "period_start": req.period_start.to_string(),
        "period_end": req.period_end.to_string(),
        "lines_imported": imported,
        "statement_hash": statement_hash.to_string(),
        "correlation_id": correlation_id,
        "imported_at": now,
    });

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_STATEMENT_IMPORTED,
        "bank_statement",
        &statement_id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    Ok(ImportResult {
        statement_id,
        lines_imported: imported,
        lines_skipped: skipped,
        errors: line_errors,
    })
}

// ============================================================================
// Helpers
// ============================================================================

async fn verify_account(pool: &PgPool, app_id: &str, account_id: Uuid) -> Result<(), ImportError> {
    let status = repo::fetch_account_status(pool, app_id, account_id).await?;
    match status {
        None => Err(ImportError::AccountNotFound(account_id)),
        Some(s) if s != AccountStatus::Active => Err(ImportError::AccountNotActive),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
#[path = "import_test.rs"]
mod tests;

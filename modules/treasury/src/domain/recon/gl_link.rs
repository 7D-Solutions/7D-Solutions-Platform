//! GL reconciliation linkage — links bank transactions to GL journal entries.
//!
//! Treasury remains read-only against GL. This module stores soft references
//! (gl_entry_id) on recon_matches, and provides unmatched queries for both
//! bank transactions (no GL link) and GL entries (caller-supplied IDs not linked).

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::ReconMatch;
use super::ReconError;
use crate::outbox::enqueue_event_tx;

const EVT_RECON_GL_LINKED: &str = "recon.gl_linked";

// ============================================================================
// Request / response types
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LinkToGlRequest {
    pub bank_transaction_id: Uuid,
    pub gl_entry_id: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnmatchedBankTxnGl {
    pub id: Uuid,
    pub account_id: Uuid,
    pub transaction_date: chrono::NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub reference: Option<String>,
    /// If matched to a statement line but not yet linked to GL
    pub has_statement_match: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UnmatchedGlRequest {
    pub gl_entry_ids: Vec<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnmatchedGlResult {
    pub provided: usize,
    pub linked: usize,
    pub unmatched_gl_entry_ids: Vec<i64>,
}

// ============================================================================
// Link a bank transaction to a GL entry
// ============================================================================

/// Idempotent: if this exact (bank_transaction_id, gl_entry_id) link already
/// exists as an active match, returns it without inserting a duplicate.
/// If the bank_txn already has an active match (from statement-line recon),
/// updates that match to include the GL entry reference.
/// Otherwise creates a GL-only match row.
pub async fn link_bank_txn_to_gl(
    pool: &PgPool,
    app_id: &str,
    req: &LinkToGlRequest,
    actor: &str,
    correlation_id: &str,
) -> Result<ReconMatch, ReconError> {
    // Guard: verify bank transaction exists
    let txn_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM treasury_bank_transactions WHERE id = $1 AND app_id = $2)",
    )
    .bind(req.bank_transaction_id)
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    if !txn_exists {
        return Err(ReconError::TransactionNotFound(req.bank_transaction_id));
    }

    // Idempotency: check if this exact link already exists
    let existing: Option<ReconMatch> = sqlx::query_as(
        r#"
        SELECT * FROM treasury_recon_matches
        WHERE bank_transaction_id = $1 AND gl_entry_id = $2
          AND superseded_by IS NULL AND app_id = $3
        "#,
    )
    .bind(req.bank_transaction_id)
    .bind(req.gl_entry_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    if let Some(m) = existing {
        return Ok(m);
    }

    // Check for an active match on this bank_txn that has no GL link yet
    let active_no_gl: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM treasury_recon_matches
        WHERE bank_transaction_id = $1 AND gl_entry_id IS NULL
          AND superseded_by IS NULL AND app_id = $2
        "#,
    )
    .bind(req.bank_transaction_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    let mut tx = pool.begin().await?;

    let match_id = if let Some(existing_id) = active_no_gl {
        // Update existing match to add GL reference
        sqlx::query(
            r#"
            UPDATE treasury_recon_matches
            SET gl_entry_id = $1, updated_at = NOW()
            WHERE id = $2
            "#,
        )
        .bind(req.gl_entry_id)
        .bind(existing_id)
        .execute(&mut *tx)
        .await?;
        existing_id
    } else {
        // Create a GL-only match (no statement_line_id)
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO treasury_recon_matches
                (id, app_id, bank_transaction_id, gl_entry_id, match_type,
                 matched_by, status, matched_at, created_at, updated_at)
            VALUES ($1, $2, $3, $4, 'manual', $5, 'confirmed', $6, $6, $6)
            "#,
        )
        .bind(id)
        .bind(app_id)
        .bind(req.bank_transaction_id)
        .bind(req.gl_entry_id)
        .bind(actor)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        id
    };

    // Outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "app_id": app_id,
        "match_id": match_id,
        "bank_transaction_id": req.bank_transaction_id,
        "gl_entry_id": req.gl_entry_id,
        "actor": actor,
        "correlation_id": correlation_id,
        "linked_at": Utc::now(),
    });
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_RECON_GL_LINKED,
        "recon",
        &match_id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    // Return the match row
    let m = sqlx::query_as::<_, ReconMatch>("SELECT * FROM treasury_recon_matches WHERE id = $1")
        .bind(match_id)
        .fetch_one(pool)
        .await?;
    Ok(m)
}

// ============================================================================
// Unmatched bank transactions (no GL link)
// ============================================================================

/// Bank transactions that have no active recon match with gl_entry_id set.
/// Includes both fully unmatched txns and those matched to statement lines
/// but not yet linked to GL.
pub async fn unmatched_bank_txns_for_gl(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedBankTxnGl>, ReconError> {
    let rows = sqlx::query_as::<_, UnmatchedBankTxnGlRow>(
        r#"
        SELECT t.id, t.account_id, t.transaction_date, t.amount_minor,
               t.currency, t.description, t.reference,
               EXISTS(
                   SELECT 1 FROM treasury_recon_matches sm
                   WHERE sm.bank_transaction_id = t.id
                     AND sm.superseded_by IS NULL
                     AND sm.statement_line_id IS NOT NULL
               ) AS has_statement_match
        FROM treasury_bank_transactions t
        WHERE t.app_id = $1 AND t.account_id = $2
          AND NOT EXISTS(
              SELECT 1 FROM treasury_recon_matches gl
              WHERE gl.bank_transaction_id = t.id
                AND gl.superseded_by IS NULL
                AND gl.gl_entry_id IS NOT NULL
          )
        ORDER BY t.transaction_date, t.id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| UnmatchedBankTxnGl {
            id: r.id,
            account_id: r.account_id,
            transaction_date: r.transaction_date,
            amount_minor: r.amount_minor,
            currency: r.currency,
            description: r.description,
            reference: r.reference,
            has_statement_match: r.has_statement_match,
        })
        .collect())
}

#[derive(Debug, sqlx::FromRow)]
struct UnmatchedBankTxnGlRow {
    id: Uuid,
    account_id: Uuid,
    transaction_date: chrono::NaiveDate,
    amount_minor: i64,
    currency: String,
    description: Option<String>,
    reference: Option<String>,
    has_statement_match: bool,
}

// ============================================================================
// Unmatched GL entries (caller provides IDs, we filter out linked ones)
// ============================================================================

/// Given a set of GL entry IDs, return those that are NOT linked to any
/// active recon match. Treasury never queries GL directly — the caller
/// provides the candidate IDs.
pub async fn unmatched_gl_entries(
    pool: &PgPool,
    app_id: &str,
    gl_entry_ids: &[i64],
) -> Result<UnmatchedGlResult, ReconError> {
    if gl_entry_ids.is_empty() {
        return Ok(UnmatchedGlResult {
            provided: 0,
            linked: 0,
            unmatched_gl_entry_ids: vec![],
        });
    }

    let linked_ids: Vec<i64> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT gl_entry_id
        FROM treasury_recon_matches
        WHERE app_id = $1
          AND gl_entry_id = ANY($2)
          AND superseded_by IS NULL
        "#,
    )
    .bind(app_id)
    .bind(gl_entry_ids)
    .fetch_all(pool)
    .await?;

    let linked_set: std::collections::HashSet<i64> = linked_ids.into_iter().collect();
    let unmatched: Vec<i64> = gl_entry_ids
        .iter()
        .filter(|id| !linked_set.contains(id))
        .copied()
        .collect();

    Ok(UnmatchedGlResult {
        provided: gl_entry_ids.len(),
        linked: linked_set.len(),
        unmatched_gl_entry_ids: unmatched,
    })
}

#[cfg(test)]
#[path = "gl_link_tests.rs"]
mod tests;

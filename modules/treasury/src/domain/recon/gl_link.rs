//! GL reconciliation linkage — links bank transactions to GL journal entries.
//!
//! Treasury remains read-only against GL. This module stores soft references
//! (gl_entry_id) on recon_matches, and provides unmatched queries for both
//! bank transactions (no GL link) and GL entries (caller-supplied IDs not linked).

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::gl_link_repo;
use super::models::ReconMatch;
use super::ReconError;
use crate::outbox::enqueue_event_tx;

const EVT_RECON_GL_LINKED: &str = "recon.gl_linked";

// ============================================================================
// Request / response types
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct LinkToGlRequest {
    pub bank_transaction_id: Uuid,
    pub gl_entry_id: i64,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
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

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct UnmatchedGlRequest {
    pub gl_entry_ids: Vec<i64>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct UnmatchedGlResult {
    pub provided: usize,
    pub linked: usize,
    pub unmatched_gl_entry_ids: Vec<i64>,
}

/// Response from the GL link endpoint.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct GlLinkResponse {
    pub match_id: Uuid,
    pub bank_transaction_id: Uuid,
    pub gl_entry_id: Option<i64>,
    pub status: super::models::ReconMatchStatus,
    pub match_type: super::models::ReconMatchType,
    pub matched_at: chrono::DateTime<chrono::Utc>,
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
    if !gl_link_repo::txn_exists(pool, app_id, req.bank_transaction_id).await? {
        return Err(ReconError::TransactionNotFound(req.bank_transaction_id));
    }

    // Idempotency: check if this exact link already exists
    if let Some(m) =
        gl_link_repo::find_existing_gl_link(pool, app_id, req.bank_transaction_id, req.gl_entry_id)
            .await?
    {
        return Ok(m);
    }

    // Check for an active match on this bank_txn that has no GL link yet
    let active_no_gl =
        gl_link_repo::find_active_match_without_gl(pool, app_id, req.bank_transaction_id).await?;

    let mut tx = pool.begin().await?;

    let match_id = if let Some(existing_id) = active_no_gl {
        // Update existing match to add GL reference
        gl_link_repo::update_match_with_gl_entry(&mut tx, existing_id, req.gl_entry_id).await?;
        existing_id
    } else {
        // Create a GL-only match (no statement_line_id)
        gl_link_repo::insert_gl_only_match(
            &mut tx,
            app_id,
            req.bank_transaction_id,
            req.gl_entry_id,
            actor,
        )
        .await?
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
    let m = super::repo::fetch_match(pool, match_id)
        .await?
        .ok_or(ReconError::MatchNotFound(match_id))?;
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
    let rows = gl_link_repo::unmatched_bank_txns_rows(pool, app_id, account_id).await?;
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

    let linked_ids = gl_link_repo::linked_gl_entry_ids(pool, app_id, gl_entry_ids).await?;

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

//! Reconciliation service — auto-match and manual-match with Guard→Mutation→Outbox.
//!
//! All matches are append-only. A rematch supersedes the prior active match
//! for that statement line (sets `superseded_by`) then inserts a new row.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::engine;
use super::models::*;
use super::repo;
use super::strategies::credit_card::CreditCardStrategy;
use super::ReconError;
use crate::domain::accounts::AccountType;
use crate::outbox::enqueue_event_tx;

const EVT_RECON_AUTO_MATCHED: &str = "recon.auto_matched";
const EVT_RECON_MANUAL_MATCHED: &str = "recon.manual_matched";

// ============================================================================
// Auto-match
// ============================================================================

pub async fn run_auto_match(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    correlation_id: &str,
) -> Result<AutoMatchResult, ReconError> {
    // Guard: fetch unmatched statement lines and payment txns for this account
    let stmt_lines = repo::fetch_unmatched_statement_lines(pool, app_id, account_id).await?;
    let pay_txns = repo::fetch_unmatched_payment_txns(pool, app_id, account_id).await?;

    // Determine account type to select matching strategy
    let account_type = repo::fetch_account_type(pool, app_id, account_id).await?;
    let candidates = match account_type {
        Some(AccountType::CreditCard) => {
            engine::auto_match_with_strategy(&stmt_lines, &pay_txns, &CreditCardStrategy)
        }
        _ => engine::auto_match(&stmt_lines, &pay_txns),
    };
    let matches_created = candidates.len();

    // Mutation + Outbox: insert each match within a transaction
    if !candidates.is_empty() {
        let mut tx = pool.begin().await?;

        for c in &candidates {
            repo::insert_match_tx(
                &mut tx,
                app_id,
                c.statement_line.id,
                c.bank_transaction.id,
                ReconMatchType::Auto,
                Some(c.confidence),
                Some("auto-engine"),
            )
            .await?;

            // Mark both sides as matched
            repo::mark_txn_matched(&mut tx, app_id, c.statement_line.id).await?;
            repo::mark_txn_matched(&mut tx, app_id, c.bank_transaction.id).await?;
        }

        let event_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "app_id": app_id,
            "account_id": account_id,
            "matches_created": matches_created,
            "correlation_id": correlation_id,
            "matched_at": Utc::now(),
        });
        enqueue_event_tx(
            &mut tx,
            event_id,
            EVT_RECON_AUTO_MATCHED,
            "recon",
            &account_id.to_string(),
            &payload,
        )
        .await?;

        tx.commit().await?;
    }

    let remaining_lines = stmt_lines.len() - matches_created;
    let remaining_txns = pay_txns.len() - matches_created;

    Ok(AutoMatchResult {
        matches_created,
        unmatched_statement_lines: remaining_lines,
        unmatched_transactions: remaining_txns,
    })
}

// ============================================================================
// Manual match
// ============================================================================

pub async fn create_manual_match(
    pool: &PgPool,
    app_id: &str,
    req: &ManualMatchRequest,
    actor: &str,
    correlation_id: &str,
) -> Result<ReconMatch, ReconError> {
    // Guard: verify both rows exist and belong to this app
    let sl = repo::fetch_txn(pool, app_id, req.statement_line_id).await?;
    let bt = repo::fetch_txn(pool, app_id, req.bank_transaction_id).await?;

    let sl = sl.ok_or(ReconError::StatementLineNotFound(req.statement_line_id))?;
    let bt = bt.ok_or(ReconError::TransactionNotFound(req.bank_transaction_id))?;

    // Guard: currency must match
    if sl.currency != bt.currency {
        return Err(ReconError::CurrencyMismatch {
            stmt_currency: sl.currency,
            txn_currency: bt.currency,
        });
    }

    // Mutation: supersede any existing active match for this statement line, insert new
    let mut tx = pool.begin().await?;

    repo::supersede_active_match(&mut tx, app_id, req.statement_line_id).await?;

    let match_id = repo::insert_match_tx(
        &mut tx,
        app_id,
        req.statement_line_id,
        req.bank_transaction_id,
        ReconMatchType::Manual,
        None,
        Some(actor),
    )
    .await?;

    repo::mark_txn_matched(&mut tx, app_id, req.statement_line_id).await?;
    repo::mark_txn_matched(&mut tx, app_id, req.bank_transaction_id).await?;

    // Outbox
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "app_id": app_id,
        "match_id": match_id,
        "statement_line_id": req.statement_line_id,
        "bank_transaction_id": req.bank_transaction_id,
        "actor": actor,
        "correlation_id": correlation_id,
        "matched_at": Utc::now(),
    });
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_RECON_MANUAL_MATCHED,
        "recon",
        &match_id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    // Fetch the newly created match
    let m = repo::fetch_match(pool, match_id)
        .await?
        .ok_or(ReconError::MatchNotFound(match_id))?;
    Ok(m)
}

// ============================================================================
// Queries (public API for HTTP handlers)
// ============================================================================

pub async fn list_matches(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    include_superseded: bool,
) -> Result<Vec<ReconMatch>, ReconError> {
    repo::list_matches(pool, app_id, account_id, include_superseded).await
}

pub async fn list_unmatched(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, ReconError> {
    repo::list_unmatched(pool, app_id, account_id).await
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

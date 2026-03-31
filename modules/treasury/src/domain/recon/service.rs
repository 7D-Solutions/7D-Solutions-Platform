//! Reconciliation service — auto-match and manual-match with Guard→Mutation→Outbox.
//!
//! All matches are append-only. A rematch supersedes the prior active match
//! for that statement line (sets `superseded_by`) then inserts a new row.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::engine;
use super::models::*;
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
    let stmt_lines = fetch_unmatched_statement_lines(pool, app_id, account_id).await?;
    let pay_txns = fetch_unmatched_payment_txns(pool, app_id, account_id).await?;

    // Determine account type to select matching strategy
    let account_type = fetch_account_type(pool, app_id, account_id).await?;
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
            insert_match_tx(
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
            mark_txn_matched(&mut tx, app_id, c.statement_line.id).await?;
            mark_txn_matched(&mut tx, app_id, c.bank_transaction.id).await?;
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
    let sl = fetch_txn(pool, app_id, req.statement_line_id).await?;
    let bt = fetch_txn(pool, app_id, req.bank_transaction_id).await?;

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

    supersede_active_match(&mut tx, app_id, req.statement_line_id).await?;

    let match_id = insert_match_tx(
        &mut tx,
        app_id,
        req.statement_line_id,
        req.bank_transaction_id,
        ReconMatchType::Manual,
        None,
        Some(actor),
    )
    .await?;

    mark_txn_matched(&mut tx, app_id, req.statement_line_id).await?;
    mark_txn_matched(&mut tx, app_id, req.bank_transaction_id).await?;

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
    let m = fetch_match(pool, match_id)
        .await?
        .ok_or(ReconError::MatchNotFound(match_id))?;
    Ok(m)
}

// ============================================================================
// Queries
// ============================================================================

pub async fn list_matches(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    include_superseded: bool,
) -> Result<Vec<ReconMatch>, ReconError> {
    let matches = if include_superseded {
        sqlx::query_as::<_, ReconMatch>(
            r#"
            SELECT m.* FROM treasury_recon_matches m
            JOIN treasury_bank_transactions t ON m.bank_transaction_id = t.id
            WHERE m.app_id = $1 AND t.account_id = $2
            ORDER BY m.created_at DESC
            "#,
        )
        .bind(app_id)
        .bind(account_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ReconMatch>(
            r#"
            SELECT m.* FROM treasury_recon_matches m
            JOIN treasury_bank_transactions t ON m.bank_transaction_id = t.id
            WHERE m.app_id = $1 AND t.account_id = $2
              AND m.superseded_by IS NULL
            ORDER BY m.created_at DESC
            "#,
        )
        .bind(app_id)
        .bind(account_id)
        .fetch_all(pool)
        .await?
    };
    Ok(matches)
}

pub async fn list_unmatched(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, ReconError> {
    let rows = sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id,
               auth_date, settle_date, merchant_name
        FROM treasury_bank_transactions
        WHERE app_id = $1 AND account_id = $2 AND status = 'unmatched'
        ORDER BY transaction_date, id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ============================================================================
// Internal helpers
// ============================================================================

async fn fetch_unmatched_statement_lines(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id,
               auth_date, settle_date, merchant_name
        FROM treasury_bank_transactions
        WHERE app_id = $1 AND account_id = $2
          AND status = 'unmatched'
          AND statement_id IS NOT NULL
        ORDER BY transaction_date, id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
}

async fn fetch_unmatched_payment_txns(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id,
               auth_date, settle_date, merchant_name
        FROM treasury_bank_transactions
        WHERE app_id = $1 AND account_id = $2
          AND status = 'unmatched'
          AND statement_id IS NULL
        ORDER BY transaction_date, id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
}

async fn fetch_txn(
    pool: &PgPool,
    app_id: &str,
    txn_id: Uuid,
) -> Result<Option<UnmatchedTxn>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id,
               auth_date, settle_date, merchant_name
        FROM treasury_bank_transactions
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(txn_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

async fn fetch_account_type(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Option<AccountType>, ReconError> {
    let row: Option<(AccountType,)> = sqlx::query_as(
        "SELECT account_type FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2",
    )
    .bind(account_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(t,)| t))
}

async fn fetch_match(pool: &PgPool, match_id: Uuid) -> Result<Option<ReconMatch>, sqlx::Error> {
    sqlx::query_as::<_, ReconMatch>("SELECT * FROM treasury_recon_matches WHERE id = $1")
        .bind(match_id)
        .fetch_optional(pool)
        .await
}

async fn insert_match_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    statement_line_id: Uuid,
    bank_transaction_id: Uuid,
    match_type: ReconMatchType,
    confidence: Option<rust_decimal::Decimal>,
    matched_by: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO treasury_recon_matches
            (id, app_id, statement_line_id, bank_transaction_id, match_type,
             confidence_score, matched_by, status, matched_at, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'confirmed', $8, $8, $8)
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(statement_line_id)
    .bind(bank_transaction_id)
    .bind(match_type)
    .bind(confidence)
    .bind(matched_by)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(id)
}

async fn supersede_active_match(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    statement_line_id: Uuid,
) -> Result<(), sqlx::Error> {
    // Find current active match for this statement line (if any)
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM treasury_recon_matches
        WHERE statement_line_id = $1 AND superseded_by IS NULL AND app_id = $2
        "#,
    )
    .bind(statement_line_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(old_id) = existing {
        // We need a placeholder — insert the new match first, then update.
        // Instead, mark the old match with a sentinel and caller updates after insert.
        // Simpler: just mark old match status as rejected and set superseded_by to
        // a well-known sentinel; the caller's insert_match_tx happens right after.
        // Actually, the unique index is on (statement_line_id) WHERE superseded_by IS NULL.
        // We need to clear the old row out of the index BEFORE inserting the new one.
        // Set superseded_by to the old match's own ID as a temporary marker.
        sqlx::query(
            r#"
            UPDATE treasury_recon_matches
            SET superseded_by = id, status = 'rejected', updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(old_id)
        .execute(&mut **tx)
        .await?;

        // Also revert the old bank_transaction_id to unmatched
        let old_txn_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT bank_transaction_id FROM treasury_recon_matches WHERE id = $1 AND app_id = $2",
        )
        .bind(old_id)
        .bind(app_id)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(txn_id) = old_txn_id {
            sqlx::query(
                "UPDATE treasury_bank_transactions SET status = 'unmatched', updated_at = NOW() WHERE id = $1 AND app_id = $2",
            )
            .bind(txn_id)
            .bind(app_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

async fn mark_txn_matched(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    txn_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE treasury_bank_transactions SET status = 'matched', updated_at = NOW() WHERE id = $1 AND app_id = $2",
    )
    .bind(txn_id)
    .bind(app_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

//! Repository layer — all SQL access for the recon domain (service + metrics).

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::accounts::AccountType;

use super::metrics::ReconSnapshot;
use super::models::{ReconMatch, ReconMatchType, UnmatchedTxn};
use super::ReconError;

// ============================================================================
// Queries (called by service)
// ============================================================================

pub async fn fetch_unmatched_statement_lines(
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

pub async fn fetch_unmatched_payment_txns(
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

pub async fn fetch_txn(
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

pub async fn fetch_account_type(
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

pub async fn fetch_match(pool: &PgPool, match_id: Uuid) -> Result<Option<ReconMatch>, sqlx::Error> {
    sqlx::query_as::<_, ReconMatch>("SELECT * FROM treasury_recon_matches WHERE id = $1")
        .bind(match_id)
        .fetch_optional(pool)
        .await
}

pub async fn insert_match_tx(
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

pub async fn supersede_active_match(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    statement_line_id: Uuid,
) -> Result<(), sqlx::Error> {
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

pub async fn mark_txn_matched(
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

// ============================================================================
// Public queries (called by service and HTTP handlers via service re-exports)
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
// Metrics snapshot (called by metrics module)
// ============================================================================

pub async fn recon_snapshot(pool: &PgPool, app_id: &str) -> Result<ReconSnapshot, sqlx::Error> {
    let matched: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_recon_matches WHERE superseded_by IS NULL AND app_id = $1",
    )
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    let unmatched_lines: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_transactions \
         WHERE status = 'unmatched' AND statement_id IS NOT NULL AND app_id = $1",
    )
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    let unmatched_txns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM treasury_bank_transactions \
         WHERE status = 'unmatched' AND statement_id IS NULL AND app_id = $1",
    )
    .bind(app_id)
    .fetch_one(pool)
    .await?;

    let total = matched + unmatched_lines;
    let match_rate = if total > 0 {
        matched as f64 / total as f64
    } else {
        0.0
    };

    Ok(ReconSnapshot {
        matched,
        unmatched_lines,
        unmatched_txns,
        match_rate,
    })
}

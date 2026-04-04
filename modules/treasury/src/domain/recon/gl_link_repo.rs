//! Repository layer — SQL access for the GL linkage sub-domain.

use chrono::Utc;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::models::ReconMatch;

// ============================================================================
// Internal row type
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
pub(super) struct UnmatchedBankTxnGlRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub transaction_date: chrono::NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub reference: Option<String>,
    pub has_statement_match: bool,
}

// ============================================================================
// Guard queries
// ============================================================================

pub async fn txn_exists(
    pool: &PgPool,
    app_id: &str,
    bank_transaction_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM treasury_bank_transactions WHERE id = $1 AND app_id = $2)",
    )
    .bind(bank_transaction_id)
    .bind(app_id)
    .fetch_one(pool)
    .await
}

pub async fn find_existing_gl_link(
    pool: &PgPool,
    app_id: &str,
    bank_transaction_id: Uuid,
    gl_entry_id: i64,
) -> Result<Option<ReconMatch>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT * FROM treasury_recon_matches
        WHERE bank_transaction_id = $1 AND gl_entry_id = $2
          AND superseded_by IS NULL AND app_id = $3
        "#,
    )
    .bind(bank_transaction_id)
    .bind(gl_entry_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

pub async fn find_active_match_without_gl(
    pool: &PgPool,
    app_id: &str,
    bank_transaction_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT id FROM treasury_recon_matches
        WHERE bank_transaction_id = $1 AND gl_entry_id IS NULL
          AND superseded_by IS NULL AND app_id = $2
        "#,
    )
    .bind(bank_transaction_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Mutation queries
// ============================================================================

pub async fn update_match_with_gl_entry(
    tx: &mut Transaction<'_, Postgres>,
    match_id: Uuid,
    gl_entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE treasury_recon_matches
        SET gl_entry_id = $1, updated_at = NOW()
        WHERE id = $2
        "#,
    )
    .bind(gl_entry_id)
    .bind(match_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_gl_only_match(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    bank_transaction_id: Uuid,
    gl_entry_id: i64,
    actor: &str,
) -> Result<Uuid, sqlx::Error> {
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
    .bind(bank_transaction_id)
    .bind(gl_entry_id)
    .bind(actor)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

// ============================================================================
// List queries
// ============================================================================

pub async fn unmatched_bank_txns_rows(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedBankTxnGlRow>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedBankTxnGlRow>(
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
    .await
}

pub async fn linked_gl_entry_ids(
    pool: &PgPool,
    app_id: &str,
    gl_entry_ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
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
    .await
}

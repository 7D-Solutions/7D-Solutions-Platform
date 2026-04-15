//! Repository layer — all SQL access for the import domain.

use chrono::{NaiveDate, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::accounts::AccountStatus;

pub async fn fetch_account_status(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Option<AccountStatus>, sqlx::Error> {
    let row: Option<(AccountStatus,)> =
        sqlx::query_as("SELECT status FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2")
            .bind(account_id)
            .bind(app_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(s,)| s))
}

pub async fn find_statement_by_hash(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    hash: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM treasury_bank_statements WHERE account_id = $1 AND statement_hash = $2 AND app_id = $3",
    )
    .bind(account_id)
    .bind(hash)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

pub async fn fetch_account_currency(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT currency FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2")
        .bind(account_id)
        .bind(app_id)
        .fetch_one(pool)
        .await
}

pub async fn insert_statement_header(
    tx: &mut Transaction<'_, Postgres>,
    statement_id: Uuid,
    app_id: &str,
    account_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    opening_balance_minor: i64,
    closing_balance_minor: i64,
    currency: &str,
    filename: Option<&str>,
    statement_hash: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO treasury_bank_statements
            (id, app_id, account_id, period_start, period_end,
             opening_balance_minor, closing_balance_minor, currency,
             status, imported_at, source_filename, statement_hash,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                'imported'::treasury_statement_status, $9, $10, $11, $9, $9)
        "#,
    )
    .bind(statement_id)
    .bind(app_id)
    .bind(account_id)
    .bind(period_start)
    .bind(period_end)
    .bind(opening_balance_minor)
    .bind(closing_balance_minor)
    .bind(currency)
    .bind(now)
    .bind(filename)
    .bind(statement_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns `true` if a new row was inserted, `false` for a duplicate (skipped).
pub async fn insert_txn_line(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    account_id: Uuid,
    statement_id: Uuid,
    date: NaiveDate,
    amount_minor: i64,
    currency: &str,
    description: &Option<String>,
    reference: Option<&str>,
    ext_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO treasury_bank_transactions
            (app_id, account_id, statement_id, transaction_date,
             amount_minor, currency, description, reference, external_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (account_id, external_id) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .bind(statement_id)
    .bind(date)
    .bind(amount_minor)
    .bind(currency)
    .bind(description.as_deref())
    .bind(reference)
    .bind(ext_id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

//! Service functions for bank transaction ingestion.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::InsertBankTxnRequest;

/// Insert a bank transaction idempotently within a caller-supplied transaction.
///
/// Uses `ON CONFLICT (account_id, external_id) DO NOTHING` so replaying
/// the same external_id is always safe.
///
/// Returns `true` if a new row was inserted, `false` for a silent duplicate.
pub async fn insert_bank_txn_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &InsertBankTxnRequest,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO treasury_bank_transactions
            (app_id, account_id, transaction_date, amount_minor, currency,
             description, reference, external_id,
             auth_date, settle_date, merchant_name, merchant_category_code)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (account_id, external_id) DO NOTHING
        "#,
    )
    .bind(&req.app_id)
    .bind(req.account_id)
    .bind(req.transaction_date)
    .bind(req.amount_minor)
    .bind(&req.currency)
    .bind(req.description.as_deref())
    .bind(req.reference.as_deref())
    .bind(&req.external_id)
    .bind(req.auth_date)
    .bind(req.settle_date)
    .bind(req.merchant_name.as_deref())
    .bind(req.merchant_category_code.as_deref())
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Mark an event as processed within a caller-supplied transaction.
///
/// Uses `ON CONFLICT (event_id) DO NOTHING` — safe to call idempotently.
pub async fn record_processed_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    processor: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(processor)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Check whether an event has already been processed (pre-flight guard).
pub async fn is_event_processed(pool: &PgPool, event_id: Uuid) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM processed_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

/// Look up the first active bank account id for an app_id.
///
/// Returns `None` if no bank account has been configured yet.
/// Consumers should warn and skip ingestion if no account is found.
pub async fn default_account_id(
    pool: &PgPool,
    app_id: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT id FROM treasury_bank_accounts
        WHERE app_id = $1 AND status = 'active'
        ORDER BY created_at ASC
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

//! Repository layer — all SQL access for the accounts domain.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    AccountError, AccountStatus, CreateBankAccountRequest, CreateCreditCardAccountRequest,
    TreasuryAccount,
};

// ============================================================================
// Reads
// ============================================================================

pub async fn get_account(
    pool: &PgPool,
    app_id: &str,
    id: Uuid,
) -> Result<Option<TreasuryAccount>, AccountError> {
    sqlx::query_as::<_, TreasuryAccount>(
        r#"
        SELECT id, app_id, account_name, account_type, institution, account_number_last4,
               routing_number, currency, current_balance_minor, status,
               credit_limit_minor, statement_closing_day, cc_network,
               metadata, created_at, updated_at
        FROM treasury_bank_accounts
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
    .map_err(AccountError::Database)
}

pub async fn list_accounts(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
) -> Result<Vec<TreasuryAccount>, AccountError> {
    if include_inactive {
        sqlx::query_as::<_, TreasuryAccount>(
            r#"
            SELECT id, app_id, account_name, account_type, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status,
                   credit_limit_minor, statement_closing_day, cc_network,
                   metadata, created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1
            ORDER BY account_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await
        .map_err(AccountError::Database)
    } else {
        sqlx::query_as::<_, TreasuryAccount>(
            r#"
            SELECT id, app_id, account_name, account_type, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status,
                   credit_limit_minor, statement_closing_day, cc_network,
                   metadata, created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1 AND status = 'active'::treasury_account_status
            ORDER BY account_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await
        .map_err(AccountError::Database)
    }
}

pub async fn count_accounts(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
) -> Result<i64, AccountError> {
    let row: (i64,) = if include_inactive {
        sqlx::query_as("SELECT COUNT(*) FROM treasury_bank_accounts WHERE app_id = $1")
            .bind(app_id)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_as(
            "SELECT COUNT(*) FROM treasury_bank_accounts WHERE app_id = $1 AND status = 'active'::treasury_account_status",
        )
        .bind(app_id)
        .fetch_one(pool)
        .await?
    };
    Ok(row.0)
}

pub async fn list_accounts_paginated(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<TreasuryAccount>, AccountError> {
    if include_inactive {
        sqlx::query_as::<_, TreasuryAccount>(
            r#"
            SELECT id, app_id, account_name, account_type, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status,
                   credit_limit_minor, statement_closing_day, cc_network,
                   metadata, created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1
            ORDER BY account_name ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(AccountError::Database)
    } else {
        sqlx::query_as::<_, TreasuryAccount>(
            r#"
            SELECT id, app_id, account_name, account_type, institution, account_number_last4,
                   routing_number, currency, current_balance_minor, status,
                   credit_limit_minor, statement_closing_day, cc_network,
                   metadata, created_at, updated_at
            FROM treasury_bank_accounts
            WHERE app_id = $1 AND status = 'active'::treasury_account_status
            ORDER BY account_name ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(AccountError::Database)
    }
}

// ============================================================================
// Writes (within transaction)
// ============================================================================

pub async fn insert_bank_account(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    id: Uuid,
    req: &CreateBankAccountRequest,
    currency: &str,
    now: DateTime<Utc>,
) -> Result<TreasuryAccount, AccountError> {
    sqlx::query_as::<_, TreasuryAccount>(
        r#"
        INSERT INTO treasury_bank_accounts (
            id, app_id, account_name, account_type, institution, account_number_last4,
            routing_number, currency, current_balance_minor, status,
            credit_limit_minor, statement_closing_day, cc_network, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'bank'::treasury_account_type, $4, $5, $6, $7, 0,
                'active'::treasury_account_status, NULL, NULL, NULL, $8, $9, $9)
        RETURNING id, app_id, account_name, account_type, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status,
                  credit_limit_minor, statement_closing_day, cc_network,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(req.account_name.trim())
    .bind(&req.institution)
    .bind(&req.account_number_last4)
    .bind(&req.routing_number)
    .bind(currency)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await
    .map_err(AccountError::Database)
}

pub async fn insert_credit_card_account(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    id: Uuid,
    req: &CreateCreditCardAccountRequest,
    currency: &str,
    now: DateTime<Utc>,
) -> Result<TreasuryAccount, AccountError> {
    sqlx::query_as::<_, TreasuryAccount>(
        r#"
        INSERT INTO treasury_bank_accounts (
            id, app_id, account_name, account_type, institution, account_number_last4,
            routing_number, currency, current_balance_minor, status,
            credit_limit_minor, statement_closing_day, cc_network, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'credit_card'::treasury_account_type, $4, $5, NULL, $6, 0,
                'active'::treasury_account_status, $7, $8, $9, $10, $11, $11)
        RETURNING id, app_id, account_name, account_type, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status,
                  credit_limit_minor, statement_closing_day, cc_network,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(req.account_name.trim())
    .bind(&req.institution)
    .bind(&req.account_number_last4)
    .bind(currency)
    .bind(req.credit_limit_minor)
    .bind(req.statement_closing_day)
    .bind(&req.cc_network)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await
    .map_err(AccountError::Database)
}

pub async fn fetch_account_for_update(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    id: Uuid,
) -> Result<Option<TreasuryAccount>, AccountError> {
    sqlx::query_as(
        r#"
        SELECT id, app_id, account_name, account_type, institution, account_number_last4,
               routing_number, currency, current_balance_minor, status,
               credit_limit_minor, statement_closing_day, cc_network,
               metadata, created_at, updated_at
        FROM treasury_bank_accounts
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AccountError::Database)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_account_fields(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    app_id: &str,
    account_name: &str,
    institution: Option<&str>,
    account_number_last4: Option<&str>,
    routing_number: Option<&str>,
    credit_limit_minor: Option<i64>,
    statement_closing_day: Option<i32>,
    cc_network: Option<&str>,
    metadata: Option<&serde_json::Value>,
    now: DateTime<Utc>,
) -> Result<TreasuryAccount, AccountError> {
    sqlx::query_as::<_, TreasuryAccount>(
        r#"
        UPDATE treasury_bank_accounts
        SET account_name = $1, institution = $2, account_number_last4 = $3,
            routing_number = $4, credit_limit_minor = $5, statement_closing_day = $6,
            cc_network = $7, metadata = $8, updated_at = $9
        WHERE id = $10 AND app_id = $11
        RETURNING id, app_id, account_name, account_type, institution, account_number_last4,
                  routing_number, currency, current_balance_minor, status,
                  credit_limit_minor, statement_closing_day, cc_network,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(account_name)
    .bind(institution)
    .bind(account_number_last4)
    .bind(routing_number)
    .bind(credit_limit_minor)
    .bind(statement_closing_day)
    .bind(cc_network)
    .bind(metadata)
    .bind(now)
    .bind(id)
    .bind(app_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(AccountError::Database)
}

pub async fn fetch_account_status(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    id: Uuid,
) -> Result<Option<AccountStatus>, AccountError> {
    let row: Option<(AccountStatus,)> =
        sqlx::query_as("SELECT status FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2")
            .bind(id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AccountError::Database)?;
    Ok(row.map(|(s,)| s))
}

pub async fn set_account_inactive(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), AccountError> {
    sqlx::query(
        r#"
        UPDATE treasury_bank_accounts
        SET status = 'inactive'::treasury_account_status, updated_at = $1
        WHERE id = $2 AND app_id = $3
        "#,
    )
    .bind(now)
    .bind(id)
    .bind(app_id)
    .execute(&mut **tx)
    .await
    .map_err(AccountError::Database)?;
    Ok(())
}

// ============================================================================
// Idempotency
// ============================================================================

pub async fn check_idempotency(pool: &PgPool, app_id: &str, key: &str) -> Result<(), AccountError> {
    let cached: Option<(serde_json::Value, i32)> = sqlx::query_as(
        "SELECT response_body, status_code FROM treasury_idempotency_keys WHERE app_id = $1 AND idempotency_key = $2 LIMIT 1",
    )
    .bind(app_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;

    if let Some((body, code)) = cached {
        return Err(AccountError::IdempotentReplay {
            status_code: code as u16,
            body,
        });
    }
    Ok(())
}

pub async fn record_idempotency(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    idempotency_key: Option<&str>,
    account: &TreasuryAccount,
    status_code: i32,
    now: DateTime<Utc>,
) -> Result<(), AccountError> {
    if let Some(key) = idempotency_key {
        let response_body = serde_json::to_value(account).unwrap_or(serde_json::Value::Null);
        let expires_at = now + chrono::Duration::hours(24);
        sqlx::query(
            r#"
            INSERT INTO treasury_idempotency_keys
                (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
            VALUES ($1, $2, '', $3, $4, $5)
            ON CONFLICT (app_id, idempotency_key) DO NOTHING
            "#,
        )
        .bind(app_id)
        .bind(key)
        .bind(response_body)
        .bind(status_code)
        .bind(expires_at)
        .execute(&mut **tx)
        .await
        .map_err(AccountError::Database)?;
    }
    Ok(())
}

// ============================================================================
// Test helpers
// ============================================================================

#[cfg(test)]
pub async fn delete_test_data(pool: &PgPool, app_id: &str) {
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'bank_account' AND aggregate_id IN \
         (SELECT id::TEXT FROM treasury_bank_accounts WHERE app_id = $1)",
    )
    .bind(app_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

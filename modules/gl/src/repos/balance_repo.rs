//! Repository for account balance operations
//!
//! Provides database access for the balance engine (Phase 11).
//! Supports upsert operations for incremental balance updates and queries for trial balance.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::repos::account_repo::{AccountType, NormalBalance};

/// Account balance model representing materialized rollup balances
#[derive(Debug, Clone, FromRow)]
pub struct AccountBalance {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub account_code: String,
    pub currency: String,
    pub debit_total_minor: i64,
    pub credit_total_minor: i64,
    pub net_balance_minor: i64,
    pub last_journal_entry_id: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Trial balance row with account metadata
/// Joins account_balances with accounts to provide complete trial balance information
#[derive(Debug, Clone, FromRow)]
pub struct TrialBalanceRow {
    // From account_balances
    pub account_code: String,
    pub currency: String,
    pub debit_total_minor: i64,
    pub credit_total_minor: i64,
    pub net_balance_minor: i64,

    // From accounts (metadata)
    pub account_name: String,
    #[sqlx(rename = "account_type")]
    pub account_type: AccountType,
    pub normal_balance: NormalBalance,
}

/// Errors that can occur during balance repository operations
#[derive(Debug, Error)]
pub enum BalanceError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Invalid balance state: {0}")]
    InvalidState(String),
}

/// Upsert a balance rollup within a transaction
///
/// This function implements the core balance update logic:
/// - INSERT if balance doesn't exist for the grain (tenant_id, period_id, account_code, currency)
/// - UPDATE (additive) if balance already exists
///
/// The operation is atomic and preserves exactly-once semantics when called within
/// a transaction alongside journal entry creation.
///
/// # Arguments
/// * `tx` - Database transaction
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period identifier
/// * `account_code` - Chart of Accounts code
/// * `currency` - ISO 4217 currency code
/// * `debit_delta` - Debit amount to add (in minor units, e.g. cents)
/// * `credit_delta` - Credit amount to add (in minor units, e.g. cents)
/// * `journal_entry_id` - The journal entry that caused this balance update
///
/// # Returns
/// The updated/inserted balance record
///
/// # Example
/// ```ignore
/// let balance = tx_upsert_rollup(
///     &mut tx,
///     "tenant_123",
///     period_id,
///     "1000", // Cash account
///     "USD",
///     10000,  // +$100.00 debit
///     0,      // no credit
///     entry_id,
/// ).await?;
/// ```
pub async fn tx_upsert_rollup(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
    account_code: &str,
    currency: &str,
    debit_delta: i64,
    credit_delta: i64,
    journal_entry_id: Uuid,
) -> Result<AccountBalance, BalanceError> {
    // Calculate net balance: debit - credit
    let net_delta = debit_delta - credit_delta;

    let balance = sqlx::query_as::<_, AccountBalance>(
        r#"
        INSERT INTO account_balances (
            tenant_id,
            period_id,
            account_code,
            currency,
            debit_total_minor,
            credit_total_minor,
            net_balance_minor,
            last_journal_entry_id,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            debit_total_minor = account_balances.debit_total_minor + EXCLUDED.debit_total_minor,
            credit_total_minor = account_balances.credit_total_minor + EXCLUDED.credit_total_minor,
            net_balance_minor = (account_balances.debit_total_minor + EXCLUDED.debit_total_minor)
                              - (account_balances.credit_total_minor + EXCLUDED.credit_total_minor),
            last_journal_entry_id = EXCLUDED.last_journal_entry_id,
            updated_at = NOW()
        RETURNING
            id,
            tenant_id,
            period_id,
            account_code,
            currency,
            debit_total_minor,
            credit_total_minor,
            net_balance_minor,
            last_journal_entry_id,
            updated_at,
            created_at
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(account_code)
    .bind(currency)
    .bind(debit_delta)
    .bind(credit_delta)
    .bind(net_delta)
    .bind(journal_entry_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(balance)
}

/// Find a balance by grain (tenant_id, period_id, account_code, currency)
///
/// Returns None if no balance exists for the specified grain.
pub async fn find_by_grain(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    account_code: &str,
    currency: &str,
) -> Result<Option<AccountBalance>, BalanceError> {
    let balance = sqlx::query_as::<_, AccountBalance>(
        r#"
        SELECT
            id,
            tenant_id,
            period_id,
            account_code,
            currency,
            debit_total_minor,
            credit_total_minor,
            net_balance_minor,
            last_journal_entry_id,
            updated_at,
            created_at
        FROM account_balances
        WHERE tenant_id = $1
          AND period_id = $2
          AND account_code = $3
          AND currency = $4
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(account_code)
    .bind(currency)
    .fetch_optional(pool)
    .await?;

    Ok(balance)
}

/// Find a balance by grain within a transaction
pub async fn find_by_grain_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
    account_code: &str,
    currency: &str,
) -> Result<Option<AccountBalance>, BalanceError> {
    let balance = sqlx::query_as::<_, AccountBalance>(
        r#"
        SELECT
            id,
            tenant_id,
            period_id,
            account_code,
            currency,
            debit_total_minor,
            credit_total_minor,
            net_balance_minor,
            last_journal_entry_id,
            updated_at,
            created_at
        FROM account_balances
        WHERE tenant_id = $1
          AND period_id = $2
          AND account_code = $3
          AND currency = $4
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(account_code)
    .bind(currency)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(balance)
}

/// Query trial balance for a tenant and period
///
/// Returns all account balances for the specified tenant and period,
/// optionally filtered by currency.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period identifier
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Vector of account balances ordered by account_code, currency
pub async fn find_trial_balance(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<Vec<AccountBalance>, BalanceError> {
    let balances = match currency {
        Some(cur) => {
            sqlx::query_as::<_, AccountBalance>(
                r#"
                SELECT
                    id,
                    tenant_id,
                    period_id,
                    account_code,
                    currency,
                    debit_total_minor,
                    credit_total_minor,
                    net_balance_minor,
                    last_journal_entry_id,
                    updated_at,
                    created_at
                FROM account_balances
                WHERE tenant_id = $1
                  AND period_id = $2
                  AND currency = $3
                ORDER BY account_code, currency
                "#,
            )
            .bind(tenant_id)
            .bind(period_id)
            .bind(cur)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, AccountBalance>(
                r#"
                SELECT
                    id,
                    tenant_id,
                    period_id,
                    account_code,
                    currency,
                    debit_total_minor,
                    credit_total_minor,
                    net_balance_minor,
                    last_journal_entry_id,
                    updated_at,
                    created_at
                FROM account_balances
                WHERE tenant_id = $1
                  AND period_id = $2
                ORDER BY account_code, currency
                "#,
            )
            .bind(tenant_id)
            .bind(period_id)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(balances)
}

/// Query trial balance with account metadata
///
/// Returns trial balance rows that join account_balances with accounts metadata.
/// This provides complete information for trial balance reporting including
/// account names, types, and normal balance directions.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period identifier
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Vector of trial balance rows ordered by account_code, currency
pub async fn find_trial_balance_with_metadata(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<Vec<TrialBalanceRow>, BalanceError> {
    let rows = match currency {
        Some(cur) => {
            sqlx::query_as::<_, TrialBalanceRow>(
                r#"
                SELECT
                    ab.account_code,
                    ab.currency,
                    ab.debit_total_minor,
                    ab.credit_total_minor,
                    ab.net_balance_minor,
                    a.name as account_name,
                    a.type as account_type,
                    a.normal_balance
                FROM account_balances ab
                INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
                WHERE ab.tenant_id = $1
                  AND ab.period_id = $2
                  AND ab.currency = $3
                  AND a.is_active = true
                ORDER BY ab.account_code, ab.currency
                "#,
            )
            .bind(tenant_id)
            .bind(period_id)
            .bind(cur)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, TrialBalanceRow>(
                r#"
                SELECT
                    ab.account_code,
                    ab.currency,
                    ab.debit_total_minor,
                    ab.credit_total_minor,
                    ab.net_balance_minor,
                    a.name as account_name,
                    a.type as account_type,
                    a.normal_balance
                FROM account_balances ab
                INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
                WHERE ab.tenant_id = $1
                  AND ab.period_id = $2
                  AND a.is_active = true
                ORDER BY ab.account_code, ab.currency
                "#,
            )
            .bind(tenant_id)
            .bind(period_id)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}

/// Find all balances for an account across periods
///
/// Useful for balance history and period-over-period analysis.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `account_code` - Chart of Accounts code
/// * `currency` - Optional currency filter
///
/// # Returns
/// Vector of account balances ordered by period (most recent first)
pub async fn find_balance_history(
    pool: &PgPool,
    tenant_id: &str,
    account_code: &str,
    currency: Option<&str>,
) -> Result<Vec<AccountBalance>, BalanceError> {
    let balances = match currency {
        Some(cur) => {
            sqlx::query_as::<_, AccountBalance>(
                r#"
                SELECT
                    ab.id,
                    ab.tenant_id,
                    ab.period_id,
                    ab.account_code,
                    ab.currency,
                    ab.debit_total_minor,
                    ab.credit_total_minor,
                    ab.net_balance_minor,
                    ab.last_journal_entry_id,
                    ab.updated_at,
                    ab.created_at
                FROM account_balances ab
                JOIN accounting_periods ap ON ab.period_id = ap.id
                WHERE ab.tenant_id = $1
                  AND ab.account_code = $2
                  AND ab.currency = $3
                ORDER BY ap.period_start DESC
                "#,
            )
            .bind(tenant_id)
            .bind(account_code)
            .bind(cur)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, AccountBalance>(
                r#"
                SELECT
                    ab.id,
                    ab.tenant_id,
                    ab.period_id,
                    ab.account_code,
                    ab.currency,
                    ab.debit_total_minor,
                    ab.credit_total_minor,
                    ab.net_balance_minor,
                    ab.last_journal_entry_id,
                    ab.updated_at,
                    ab.created_at
                FROM account_balances ab
                JOIN accounting_periods ap ON ab.period_id = ap.id
                WHERE ab.tenant_id = $1
                  AND ab.account_code = $2
                ORDER BY ap.period_start DESC, ab.currency
                "#,
            )
            .bind(tenant_id)
            .bind(account_code)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(balances)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_error_display() {
        let err = BalanceError::InvalidState("negative debit".to_string());
        assert!(err.to_string().contains("negative debit"));
    }
}

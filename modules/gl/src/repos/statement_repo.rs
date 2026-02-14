//! Statement Repository (Phase 14)
//!
//! Provides aggregated single-query operations for financial statements.
//! All queries join account_balances + accounts + periods for complete metadata.
//!
//! **Performance Contract**: 1 query per statement, no N+1, indexed path enforced.

use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::statements::{BalanceSheetRow, IncomeStatementRow, TrialBalanceRow};
use crate::repos::account_repo::{AccountType, NormalBalance};

/// Errors that can occur during statement repository operations
#[derive(Debug, Error)]
pub enum StatementError {
    #[error("Period not found: period_id={0}")]
    PeriodNotFound(Uuid),

    #[error("Invalid currency code: {0}")]
    InvalidCurrency(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================
// INTERNAL DB MODELS (with sqlx type mappings)
// ============================================================

/// Internal trial balance row with DB-specific types
#[derive(Debug, Clone, FromRow)]
struct TrialBalanceRowDb {
    pub account_code: String,
    pub account_name: String,
    #[sqlx(rename = "account_type")]
    pub account_type: AccountType,
    pub normal_balance: NormalBalance,
    pub currency: String,
    pub debit_total_minor: i64,
    pub credit_total_minor: i64,
    pub net_balance_minor: i64,
}

/// Internal income statement row with DB-specific types
#[derive(Debug, Clone, FromRow)]
struct IncomeStatementRowDb {
    pub account_code: String,
    pub account_name: String,
    #[sqlx(rename = "account_type")]
    pub account_type: AccountType,
    pub currency: String,
    pub net_balance_minor: i64,
}

/// Internal balance sheet row with DB-specific types
#[derive(Debug, Clone, FromRow)]
struct BalanceSheetRowDb {
    pub account_code: String,
    pub account_name: String,
    #[sqlx(rename = "account_type")]
    pub account_type: AccountType,
    pub currency: String,
    pub net_balance_minor: i64,
}

// ============================================================
// CONVERSION HELPERS
// ============================================================

/// Convert AccountType enum to string
fn account_type_to_string(account_type: &AccountType) -> String {
    match account_type {
        AccountType::Asset => "asset".to_string(),
        AccountType::Liability => "liability".to_string(),
        AccountType::Equity => "equity".to_string(),
        AccountType::Revenue => "revenue".to_string(),
        AccountType::Expense => "expense".to_string(),
    }
}

/// Convert NormalBalance enum to string
fn normal_balance_to_string(normal_balance: &NormalBalance) -> String {
    match normal_balance {
        NormalBalance::Debit => "debit".to_string(),
        NormalBalance::Credit => "credit".to_string(),
    }
}

// ============================================================
// PUBLIC API
// ============================================================

/// Get trial balance rows for a period
///
/// Single-query aggregation from account_balances + accounts.
/// Returns all active accounts with balances for the specified period.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Vector of trial balance rows using domain model
///
/// # Performance
/// Uses indexes: idx_account_balances_tenant_period, idx_accounts_tenant_code
/// Expected: < 150ms for 10,000 accounts (per Phase 14 spec)
pub async fn get_trial_balance_rows(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<Vec<TrialBalanceRow>, StatementError> {
    // Validate currency if provided
    if let Some(cur) = currency {
        if cur.len() != 3 || !cur.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(StatementError::InvalidCurrency(cur.to_string()));
        }
    }

    // Single query: JOIN account_balances + accounts
    let query = match currency {
        Some(_) => {
            r#"
            SELECT
                ab.account_code,
                a.name as account_name,
                a.type as account_type,
                a.normal_balance,
                ab.currency,
                ab.debit_total_minor,
                ab.credit_total_minor,
                ab.net_balance_minor
            FROM account_balances ab
            INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND ab.currency = $3
              AND a.is_active = true
            ORDER BY ab.account_code, ab.currency
            "#
        }
        None => {
            r#"
            SELECT
                ab.account_code,
                a.name as account_name,
                a.type as account_type,
                a.normal_balance,
                ab.currency,
                ab.debit_total_minor,
                ab.credit_total_minor,
                ab.net_balance_minor
            FROM account_balances ab
            INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND a.is_active = true
            ORDER BY ab.account_code, ab.currency
            "#
        }
    };

    let db_rows: Vec<TrialBalanceRowDb> = match currency {
        Some(cur) => {
            sqlx::query_as(query)
                .bind(tenant_id)
                .bind(period_id)
                .bind(cur)
                .fetch_all(pool)
                .await?
        }
        None => {
            sqlx::query_as(query)
                .bind(tenant_id)
                .bind(period_id)
                .fetch_all(pool)
                .await?
        }
    };

    // Convert DB models to domain models
    let domain_rows = db_rows
        .into_iter()
        .map(|row| TrialBalanceRow {
            account_code: row.account_code,
            account_name: row.account_name,
            account_type: account_type_to_string(&row.account_type),
            normal_balance: normal_balance_to_string(&row.normal_balance),
            currency: row.currency,
            debit_total_minor: row.debit_total_minor,
            credit_total_minor: row.credit_total_minor,
            net_balance_minor: row.net_balance_minor,
        })
        .collect();

    Ok(domain_rows)
}

/// Get income statement rows for a period
///
/// Single-query aggregation from account_balances + accounts.
/// Returns only revenue and expense accounts with balances.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Vector of income statement rows using domain model
///
/// # Performance
/// Uses indexes: idx_account_balances_tenant_period, idx_accounts_tenant_code
/// Expected: < 150ms for 10,000 accounts (per Phase 14 spec)
pub async fn get_income_statement_rows(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<Vec<IncomeStatementRow>, StatementError> {
    // Validate currency if provided
    if let Some(cur) = currency {
        if cur.len() != 3 || !cur.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(StatementError::InvalidCurrency(cur.to_string()));
        }
    }

    // Single query: JOIN account_balances + accounts, filter by revenue/expense
    let query = match currency {
        Some(_) => {
            r#"
            SELECT
                ab.account_code,
                a.name as account_name,
                a.type as account_type,
                ab.currency,
                ab.net_balance_minor
            FROM account_balances ab
            INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND ab.currency = $3
              AND a.is_active = true
              AND a.type IN ('revenue', 'expense')
            ORDER BY a.type DESC, ab.account_code, ab.currency
            "#
        }
        None => {
            r#"
            SELECT
                ab.account_code,
                a.name as account_name,
                a.type as account_type,
                ab.currency,
                ab.net_balance_minor
            FROM account_balances ab
            INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND a.is_active = true
              AND a.type IN ('revenue', 'expense')
            ORDER BY a.type DESC, ab.account_code, ab.currency
            "#
        }
    };

    let db_rows: Vec<IncomeStatementRowDb> = match currency {
        Some(cur) => {
            sqlx::query_as(query)
                .bind(tenant_id)
                .bind(period_id)
                .bind(cur)
                .fetch_all(pool)
                .await?
        }
        None => {
            sqlx::query_as(query)
                .bind(tenant_id)
                .bind(period_id)
                .fetch_all(pool)
                .await?
        }
    };

    // Convert DB models to domain models
    // For income statement: revenue is positive (credit balance), expense is negative (debit balance)
    let domain_rows = db_rows
        .into_iter()
        .map(|row| {
            let amount_minor = match row.account_type {
                AccountType::Revenue => row.net_balance_minor, // Credit balance = positive
                AccountType::Expense => -row.net_balance_minor, // Debit balance = negative
                _ => row.net_balance_minor, // Shouldn't happen due to SQL filter
            };

            IncomeStatementRow {
                account_code: row.account_code,
                account_name: row.account_name,
                account_type: account_type_to_string(&row.account_type),
                currency: row.currency,
                amount_minor,
            }
        })
        .collect();

    Ok(domain_rows)
}

/// Get balance sheet rows for a period
///
/// Single-query aggregation from account_balances + accounts.
/// Returns only asset, liability, and equity accounts with balances.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Vector of balance sheet rows using domain model
///
/// # Performance
/// Uses indexes: idx_account_balances_tenant_period, idx_accounts_tenant_code
/// Expected: < 150ms for 10,000 accounts (per Phase 14 spec)
pub async fn get_balance_sheet_rows(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<Vec<BalanceSheetRow>, StatementError> {
    // Validate currency if provided
    if let Some(cur) = currency {
        if cur.len() != 3 || !cur.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(StatementError::InvalidCurrency(cur.to_string()));
        }
    }

    // Single query: JOIN account_balances + accounts, filter by asset/liability/equity
    let query = match currency {
        Some(_) => {
            r#"
            SELECT
                ab.account_code,
                a.name as account_name,
                a.type as account_type,
                ab.currency,
                ab.net_balance_minor
            FROM account_balances ab
            INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND ab.currency = $3
              AND a.is_active = true
              AND a.type IN ('asset', 'liability', 'equity')
            ORDER BY a.type, ab.account_code, ab.currency
            "#
        }
        None => {
            r#"
            SELECT
                ab.account_code,
                a.name as account_name,
                a.type as account_type,
                ab.currency,
                ab.net_balance_minor
            FROM account_balances ab
            INNER JOIN accounts a ON a.tenant_id = ab.tenant_id AND a.code = ab.account_code
            WHERE ab.tenant_id = $1
              AND ab.period_id = $2
              AND a.is_active = true
              AND a.type IN ('asset', 'liability', 'equity')
            ORDER BY a.type, ab.account_code, ab.currency
            "#
        }
    };

    let db_rows: Vec<BalanceSheetRowDb> = match currency {
        Some(cur) => {
            sqlx::query_as(query)
                .bind(tenant_id)
                .bind(period_id)
                .bind(cur)
                .fetch_all(pool)
                .await?
        }
        None => {
            sqlx::query_as(query)
                .bind(tenant_id)
                .bind(period_id)
                .fetch_all(pool)
                .await?
        }
    };

    // Convert DB models to domain models
    let domain_rows = db_rows
        .into_iter()
        .map(|row| BalanceSheetRow {
            account_code: row.account_code,
            account_name: row.account_name,
            account_type: account_type_to_string(&row.account_type),
            currency: row.currency,
            amount_minor: row.net_balance_minor,
        })
        .collect();

    Ok(domain_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_type_conversion() {
        assert_eq!(account_type_to_string(&AccountType::Asset), "asset");
        assert_eq!(account_type_to_string(&AccountType::Liability), "liability");
        assert_eq!(account_type_to_string(&AccountType::Equity), "equity");
        assert_eq!(account_type_to_string(&AccountType::Revenue), "revenue");
        assert_eq!(account_type_to_string(&AccountType::Expense), "expense");
    }

    #[test]
    fn test_normal_balance_conversion() {
        assert_eq!(normal_balance_to_string(&NormalBalance::Debit), "debit");
        assert_eq!(normal_balance_to_string(&NormalBalance::Credit), "credit");
    }
}

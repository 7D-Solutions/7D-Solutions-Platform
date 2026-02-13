//! Trial Balance Service
//!
//! Provides read-only trial balance reporting backed by account_balances table.
//! Implements strict tenant isolation, period scoping, and multi-currency filtering.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::repos::balance_repo::{self, BalanceError};
use crate::repos::account_repo::{AccountType, NormalBalance};

/// Trial balance response with account balances and totals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialBalanceResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: Option<String>,
    pub rows: Vec<TrialBalanceRowDto>,
    pub totals: TrialBalanceTotals,
}

/// Trial balance row DTO (Data Transfer Object)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialBalanceRowDto {
    pub account_code: String,
    pub account_name: String,
    pub account_type: String,
    pub normal_balance: String,
    pub currency: String,
    pub debit_total_minor: i64,
    pub credit_total_minor: i64,
    pub net_balance_minor: i64,
}

/// Trial balance totals for verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialBalanceTotals {
    pub total_debits: i64,
    pub total_credits: i64,
    pub is_balanced: bool,
}

/// Errors that can occur during trial balance operations
#[derive(Debug, Error)]
pub enum TrialBalanceError {
    #[error("Balance repository error: {0}")]
    BalanceRepo(#[from] BalanceError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Invalid currency: {0}")]
    InvalidCurrency(String),
}

/// Get trial balance for a tenant and period
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Optional currency filter (None = all currencies)
///
/// # Returns
/// Trial balance response with rows and totals
///
/// # Example
/// ```ignore
/// let tb = get_trial_balance(
///     &pool,
///     "tenant_123",
///     period_id,
///     Some("USD"),
/// ).await?;
/// ```
pub async fn get_trial_balance(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: Option<&str>,
) -> Result<TrialBalanceResponse, TrialBalanceError> {
    // Validate inputs
    if tenant_id.is_empty() {
        return Err(TrialBalanceError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }

    if let Some(cur) = currency {
        if cur.len() != 3 || !cur.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(TrialBalanceError::InvalidCurrency(format!(
                "Currency must be 3 uppercase letters (ISO 4217): {}",
                cur
            )));
        }
    }

    // Query trial balance from repository (backed by account_balances + accounts join)
    let rows = balance_repo::find_trial_balance_with_metadata(pool, tenant_id, period_id, currency)
        .await?;

    // Convert repository models to DTOs
    let row_dtos: Vec<TrialBalanceRowDto> = rows
        .into_iter()
        .map(|row| TrialBalanceRowDto {
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

    // Calculate totals
    let totals = calculate_totals(&row_dtos);

    Ok(TrialBalanceResponse {
        tenant_id: tenant_id.to_string(),
        period_id,
        currency: currency.map(|c| c.to_string()),
        rows: row_dtos,
        totals,
    })
}

/// Convert AccountType to string
fn account_type_to_string(account_type: &AccountType) -> String {
    match account_type {
        AccountType::Asset => "asset".to_string(),
        AccountType::Liability => "liability".to_string(),
        AccountType::Equity => "equity".to_string(),
        AccountType::Revenue => "revenue".to_string(),
        AccountType::Expense => "expense".to_string(),
    }
}

/// Convert NormalBalance to string
fn normal_balance_to_string(normal_balance: &NormalBalance) -> String {
    match normal_balance {
        NormalBalance::Debit => "debit".to_string(),
        NormalBalance::Credit => "credit".to_string(),
    }
}

/// Calculate trial balance totals
///
/// Sums all debit and credit totals and checks if they balance.
fn calculate_totals(rows: &[TrialBalanceRowDto]) -> TrialBalanceTotals {
    let total_debits: i64 = rows.iter().map(|r| r.debit_total_minor).sum();
    let total_credits: i64 = rows.iter().map(|r| r.credit_total_minor).sum();
    let is_balanced = total_debits == total_credits;

    TrialBalanceTotals {
        total_debits,
        total_credits,
        is_balanced,
    }
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

    #[test]
    fn test_calculate_totals_balanced() {
        let rows = vec![
            TrialBalanceRowDto {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 100000,
                credit_total_minor: 0,
                net_balance_minor: 100000,
            },
            TrialBalanceRowDto {
                account_code: "4000".to_string(),
                account_name: "Revenue".to_string(),
                account_type: "revenue".to_string(),
                normal_balance: "credit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 0,
                credit_total_minor: 100000,
                net_balance_minor: -100000,
            },
        ];

        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_debits, 100000);
        assert_eq!(totals.total_credits, 100000);
        assert!(totals.is_balanced);
    }

    #[test]
    fn test_calculate_totals_unbalanced() {
        let rows = vec![
            TrialBalanceRowDto {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 150000,
                credit_total_minor: 0,
                net_balance_minor: 150000,
            },
            TrialBalanceRowDto {
                account_code: "4000".to_string(),
                account_name: "Revenue".to_string(),
                account_type: "revenue".to_string(),
                normal_balance: "credit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 0,
                credit_total_minor: 100000,
                net_balance_minor: -100000,
            },
        ];

        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_debits, 150000);
        assert_eq!(totals.total_credits, 100000);
        assert!(!totals.is_balanced);
    }
}

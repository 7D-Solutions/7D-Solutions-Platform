//! Trial Balance Service (Phase 14.4)
//!
//! Provides read-only trial balance reporting backed by statement_repo.
//! Implements strict tenant isolation, period scoping, and required currency filtering.
//!
//! **Performance**: Single-query aggregation from account_balances + accounts + periods.
//! **Currency Policy**: Required parameter - one statement per currency (no multi-currency aggregation).
//! **Validation**: Totals must balance (sum debits == sum credits) or assertion fails.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::statements::{TrialBalanceRow, StatementTotals};
use crate::repos::statement_repo::{self, StatementError};

/// Trial balance response with account balances and totals
///
/// **Currency Policy**: Single-currency only (currency is required parameter).
/// **Balance Guarantee**: totals.is_balanced MUST be true or data is invalid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialBalanceResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String, // Required - no Optional wrapper
    pub rows: Vec<TrialBalanceRow>,
    pub totals: StatementTotals,
}

/// Errors that can occur during trial balance operations
#[derive(Debug, Error)]
pub enum TrialBalanceError {
    #[error("Statement repository error: {0}")]
    StatementRepo(#[from] StatementError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Trial balance does not balance: debits={debits}, credits={credits}")]
    Unbalanced { debits: i64, credits: i64 },
}

/// Get trial balance for a tenant and period
///
/// **Phase 14 Service**: Assembles domain + repo into service layer.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `currency` - Currency code (ISO 4217, required) - one statement per currency
///
/// # Returns
/// Trial balance response with rows and totals
///
/// # Errors
/// Returns `Unbalanced` if total debits != total credits (data integrity violation)
/// Returns `StatementRepo` error if period not found or database error
///
/// # Example
/// ```ignore
/// let tb = get_trial_balance(
///     &pool,
///     "tenant_123",
///     period_id,
///     "USD",  // Required - no Optional wrapper
/// ).await?;
///
/// // Guaranteed: tb.totals.is_balanced == true (or error was returned)
/// assert!(tb.totals.is_balanced);
/// ```
pub async fn get_trial_balance(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,  // Changed from Option<&str> - currency is now required
) -> Result<TrialBalanceResponse, TrialBalanceError> {
    // Validate tenant_id
    if tenant_id.is_empty() {
        return Err(TrialBalanceError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }

    // Query trial balance from statement repository
    // Currency validation happens in statement_repo (ISO 4217 format check)
    let rows = statement_repo::get_trial_balance_rows(pool, tenant_id, period_id, currency)
        .await?;

    // Calculate totals
    let totals = calculate_totals(&rows);

    // Validate balance (acceptance criteria: assertion fails if imbalance)
    if !totals.is_balanced {
        return Err(TrialBalanceError::Unbalanced {
            debits: totals.total_debits,
            credits: totals.total_credits,
        });
    }

    Ok(TrialBalanceResponse {
        tenant_id: tenant_id.to_string(),
        period_id,
        currency: currency.to_string(),
        rows,
        totals,
    })
}

/// Calculate trial balance totals
///
/// Sums all debit and credit totals and checks if they balance.
/// Per accounting principles: total debits MUST equal total credits.
fn calculate_totals(rows: &[TrialBalanceRow]) -> StatementTotals {
    let total_debits: i64 = rows.iter().map(|r| r.debit_total_minor).sum();
    let total_credits: i64 = rows.iter().map(|r| r.credit_total_minor).sum();
    let is_balanced = total_debits == total_credits;

    StatementTotals {
        total_debits,
        total_credits,
        is_balanced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_totals_balanced() {
        let rows = vec![
            TrialBalanceRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 100000,
                credit_total_minor: 0,
                net_balance_minor: 100000,
            },
            TrialBalanceRow {
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
            TrialBalanceRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 150000,
                credit_total_minor: 0,
                net_balance_minor: 150000,
            },
            TrialBalanceRow {
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

    #[test]
    fn test_calculate_totals_zero() {
        let rows: Vec<TrialBalanceRow> = vec![];
        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_debits, 0);
        assert_eq!(totals.total_credits, 0);
        assert!(totals.is_balanced, "Empty trial balance should be balanced");
    }

    #[test]
    fn test_calculate_totals_deterministic_ordering() {
        // Verify totals calculation is order-independent
        let rows1 = vec![
            TrialBalanceRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 50000,
                credit_total_minor: 0,
                net_balance_minor: 50000,
            },
            TrialBalanceRow {
                account_code: "4000".to_string(),
                account_name: "Revenue".to_string(),
                account_type: "revenue".to_string(),
                normal_balance: "credit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 0,
                credit_total_minor: 50000,
                net_balance_minor: -50000,
            },
        ];

        // Same data, reversed order
        let rows2 = vec![
            TrialBalanceRow {
                account_code: "4000".to_string(),
                account_name: "Revenue".to_string(),
                account_type: "revenue".to_string(),
                normal_balance: "credit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 0,
                credit_total_minor: 50000,
                net_balance_minor: -50000,
            },
            TrialBalanceRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 50000,
                credit_total_minor: 0,
                net_balance_minor: 50000,
            },
        ];

        let totals1 = calculate_totals(&rows1);
        let totals2 = calculate_totals(&rows2);

        assert_eq!(totals1.total_debits, totals2.total_debits);
        assert_eq!(totals1.total_credits, totals2.total_credits);
        assert_eq!(totals1.is_balanced, totals2.is_balanced);
    }
}

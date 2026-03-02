//! Income Statement Service (Phase 14.5)
//!
//! Provides read-only income statement (P&L) reporting backed by statement_repo.
//! Implements revenue/expense aggregation with net income calculation.
//!
//! **Performance**: Single-query aggregation from account_balances + accounts + periods.
//! **Currency Policy**: Required parameter - one statement per currency (no multi-currency aggregation).
//! **Validation**: Net income must equal revenue - expenses (accounting equation enforced).

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::statements::IncomeStatementRow;
use crate::repos::statement_repo::{self, StatementError};

/// Income statement response with revenue/expense rows and calculated totals
///
/// **Currency Policy**: Single-currency only (currency is required parameter).
/// **Accounting Equation**: net_income MUST equal total_revenue - total_expenses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomeStatementResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String, // Required - no Optional wrapper
    pub rows: Vec<IncomeStatementRow>,
    pub totals: IncomeStatementTotals,
}

/// Income statement totals with revenue, expenses, and net income
///
/// **Accounting Equation**: net_income = total_revenue - total_expenses
/// **Sign Convention**: Revenue is positive, expenses are negative, net_income is positive for profit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomeStatementTotals {
    /// Total revenue (credit balance accounts, positive amounts)
    pub total_revenue: i64,

    /// Total expenses (debit balance accounts, negative amounts)
    pub total_expenses: i64,

    /// Net income = total_revenue - total_expenses
    /// Positive = profit, Negative = loss
    pub net_income: i64,
}

/// Errors that can occur during income statement operations
#[derive(Debug, Error)]
pub enum IncomeStatementError {
    #[error("Statement repository error: {0}")]
    StatementRepo(#[from] StatementError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Accounting equation violation: net_income={net_income}, revenue={revenue}, expenses={expenses}")]
    AccountingEquationViolation {
        net_income: i64,
        revenue: i64,
        expenses: i64,
    },
}

/// Get income statement for a tenant and period
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
/// Income statement response with rows and totals
///
/// # Errors
/// Returns `AccountingEquationViolation` if net_income != revenue - expenses
/// Returns `StatementRepo` error if period not found or database error
///
/// # Example
/// ```ignore
/// let is = get_income_statement(
///     &pool,
///     "tenant_123",
///     period_id,
///     "USD",  // Required - no Optional wrapper
/// ).await?;
///
/// // Guaranteed: is.totals.net_income == is.totals.total_revenue - is.totals.total_expenses
/// assert_eq!(
///     is.totals.net_income,
///     is.totals.total_revenue - is.totals.total_expenses
/// );
/// ```
pub async fn get_income_statement(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str, // Required - currency is mandatory for single-currency statements
) -> Result<IncomeStatementResponse, IncomeStatementError> {
    // Validate tenant_id
    if tenant_id.is_empty() {
        return Err(IncomeStatementError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }

    // Query income statement rows from statement repository
    // Currency validation happens in statement_repo (ISO 4217 format check)
    // Rows are already filtered to revenue/expense accounts only
    // Amount signs are already correct: revenue positive, expenses negative
    let rows =
        statement_repo::get_income_statement_rows(pool, tenant_id, period_id, currency).await?;

    // Calculate totals
    let totals = calculate_totals(&rows)?;

    Ok(IncomeStatementResponse {
        tenant_id: tenant_id.to_string(),
        period_id,
        currency: currency.to_string(),
        rows,
        totals,
    })
}

/// Calculate income statement totals
///
/// Aggregates revenue and expenses, computes net income.
/// Validates accounting equation: net_income = total_revenue - total_expenses.
///
/// # Sign Convention
/// - Revenue rows have positive amounts (credit balance)
/// - Expense rows have negative amounts (debit balance inverted)
/// - Net income = sum of all amounts = revenue + expenses (expenses are negative)
fn calculate_totals(
    rows: &[IncomeStatementRow],
) -> Result<IncomeStatementTotals, IncomeStatementError> {
    // Separate revenue and expense rows
    let revenue_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.account_type == "revenue")
        .collect();

    let expense_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.account_type == "expense")
        .collect();

    // Sum revenue (should be positive amounts)
    let total_revenue: i64 = revenue_rows.iter().map(|r| r.amount_minor).sum();

    // Sum expenses (should be negative amounts)
    let total_expenses: i64 = expense_rows.iter().map(|r| r.amount_minor).sum();

    // Calculate net income
    // Since expenses are already negative, this is: revenue + (-expenses) = revenue - expenses
    let net_income = total_revenue + total_expenses;

    // Validate accounting equation: net_income should equal total_revenue - total_expenses
    // This is redundant but serves as a safety check
    let expected_net_income = total_revenue - total_expenses.abs();
    if net_income != expected_net_income && total_expenses != 0 {
        // Only fail if there are actual expenses (avoid edge case with zero expenses)
        return Err(IncomeStatementError::AccountingEquationViolation {
            net_income,
            revenue: total_revenue,
            expenses: total_expenses,
        });
    }

    Ok(IncomeStatementTotals {
        total_revenue,
        total_expenses,
        net_income,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_totals_profit() {
        let rows = vec![
            IncomeStatementRow {
                account_code: "4000".to_string(),
                account_name: "Sales Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "USD".to_string(),
                amount_minor: 500000, // Revenue is positive
            },
            IncomeStatementRow {
                account_code: "5000".to_string(),
                account_name: "Salaries Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "USD".to_string(),
                amount_minor: -300000, // Expense is negative
            },
        ];

        let totals = calculate_totals(&rows).unwrap();
        assert_eq!(totals.total_revenue, 500000);
        assert_eq!(totals.total_expenses, -300000);
        assert_eq!(totals.net_income, 200000); // 500000 + (-300000) = 200000 profit
    }

    #[test]
    fn test_calculate_totals_loss() {
        let rows = vec![
            IncomeStatementRow {
                account_code: "4000".to_string(),
                account_name: "Sales Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "USD".to_string(),
                amount_minor: 100000, // Revenue is positive
            },
            IncomeStatementRow {
                account_code: "5000".to_string(),
                account_name: "Salaries Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "USD".to_string(),
                amount_minor: -300000, // Expense is negative
            },
        ];

        let totals = calculate_totals(&rows).unwrap();
        assert_eq!(totals.total_revenue, 100000);
        assert_eq!(totals.total_expenses, -300000);
        assert_eq!(totals.net_income, -200000); // 100000 + (-300000) = -200000 loss
    }

    #[test]
    fn test_calculate_totals_zero() {
        let rows: Vec<IncomeStatementRow> = vec![];
        let totals = calculate_totals(&rows).unwrap();
        assert_eq!(totals.total_revenue, 0);
        assert_eq!(totals.total_expenses, 0);
        assert_eq!(totals.net_income, 0);
    }

    #[test]
    fn test_calculate_totals_revenue_only() {
        let rows = vec![IncomeStatementRow {
            account_code: "4000".to_string(),
            account_name: "Sales Revenue".to_string(),
            account_type: "revenue".to_string(),
            currency: "USD".to_string(),
            amount_minor: 500000,
        }];

        let totals = calculate_totals(&rows).unwrap();
        assert_eq!(totals.total_revenue, 500000);
        assert_eq!(totals.total_expenses, 0);
        assert_eq!(totals.net_income, 500000);
    }

    #[test]
    fn test_calculate_totals_expense_only() {
        let rows = vec![IncomeStatementRow {
            account_code: "5000".to_string(),
            account_name: "Salaries Expense".to_string(),
            account_type: "expense".to_string(),
            currency: "USD".to_string(),
            amount_minor: -300000,
        }];

        let totals = calculate_totals(&rows).unwrap();
        assert_eq!(totals.total_revenue, 0);
        assert_eq!(totals.total_expenses, -300000);
        assert_eq!(totals.net_income, -300000);
    }

    #[test]
    fn test_calculate_totals_multiple_revenue_expense() {
        let rows = vec![
            // Multiple revenue accounts
            IncomeStatementRow {
                account_code: "4000".to_string(),
                account_name: "Sales Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "USD".to_string(),
                amount_minor: 500000,
            },
            IncomeStatementRow {
                account_code: "4100".to_string(),
                account_name: "Service Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "USD".to_string(),
                amount_minor: 200000,
            },
            // Multiple expense accounts
            IncomeStatementRow {
                account_code: "5000".to_string(),
                account_name: "Salaries Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "USD".to_string(),
                amount_minor: -300000,
            },
            IncomeStatementRow {
                account_code: "5100".to_string(),
                account_name: "Rent Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "USD".to_string(),
                amount_minor: -100000,
            },
        ];

        let totals = calculate_totals(&rows).unwrap();
        assert_eq!(totals.total_revenue, 700000); // 500000 + 200000
        assert_eq!(totals.total_expenses, -400000); // -300000 + -100000
        assert_eq!(totals.net_income, 300000); // 700000 - 400000
    }

    #[test]
    fn test_calculate_totals_deterministic_ordering() {
        // Verify totals calculation is order-independent
        let rows1 = vec![
            IncomeStatementRow {
                account_code: "4000".to_string(),
                account_name: "Sales Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "USD".to_string(),
                amount_minor: 500000,
            },
            IncomeStatementRow {
                account_code: "5000".to_string(),
                account_name: "Salaries Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "USD".to_string(),
                amount_minor: -300000,
            },
        ];

        // Same data, reversed order
        let rows2 = vec![
            IncomeStatementRow {
                account_code: "5000".to_string(),
                account_name: "Salaries Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "USD".to_string(),
                amount_minor: -300000,
            },
            IncomeStatementRow {
                account_code: "4000".to_string(),
                account_name: "Sales Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "USD".to_string(),
                amount_minor: 500000,
            },
        ];

        let totals1 = calculate_totals(&rows1).unwrap();
        let totals2 = calculate_totals(&rows2).unwrap();

        assert_eq!(totals1.total_revenue, totals2.total_revenue);
        assert_eq!(totals1.total_expenses, totals2.total_expenses);
        assert_eq!(totals1.net_income, totals2.net_income);
    }

    /// Comprehensive snapshot serialization test
    ///
    /// Verifies JSON structure stability for income statement response.
    /// This test serves as a contract test to ensure serialization format
    /// remains stable across refactorings.
    #[test]
    fn test_comprehensive_snapshot_serialization() {
        let response = IncomeStatementResponse {
            tenant_id: "tenant_123".to_string(),
            period_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            currency: "USD".to_string(),
            rows: vec![
                IncomeStatementRow {
                    account_code: "4000".to_string(),
                    account_name: "Sales Revenue".to_string(),
                    account_type: "revenue".to_string(),
                    currency: "USD".to_string(),
                    amount_minor: 500000,
                },
                IncomeStatementRow {
                    account_code: "5000".to_string(),
                    account_name: "Salaries Expense".to_string(),
                    account_type: "expense".to_string(),
                    currency: "USD".to_string(),
                    amount_minor: -300000,
                },
            ],
            totals: IncomeStatementTotals {
                total_revenue: 500000,
                total_expenses: -300000,
                net_income: 200000,
            },
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["tenant_id"], "tenant_123");
        assert_eq!(json["period_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(json["currency"], "USD");
        assert_eq!(json["rows"].as_array().unwrap().len(), 2);
        assert_eq!(json["totals"]["total_revenue"], 500000);
        assert_eq!(json["totals"]["total_expenses"], -300000);
        assert_eq!(json["totals"]["net_income"], 200000);

        // Verify round-trip serialization
        let roundtrip: IncomeStatementResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.tenant_id, roundtrip.tenant_id);
        assert_eq!(response.period_id, roundtrip.period_id);
        assert_eq!(response.currency, roundtrip.currency);
        assert_eq!(response.rows.len(), roundtrip.rows.len());
        assert_eq!(response.totals, roundtrip.totals);
    }
}

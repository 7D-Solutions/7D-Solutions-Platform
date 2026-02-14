//! Balance Sheet Service (Phase 14.6)
//!
//! Provides read-only balance sheet reporting backed by statement_repo.
//! Implements strict tenant isolation, period scoping, and required currency filtering.
//!
//! **Performance**: Single-query aggregation from account_balances + accounts + periods.
//! **Currency Policy**: Required parameter - one statement per currency (no multi-currency aggregation).
//! **Validation**: Assets MUST equal Liabilities + Equity or assertion fails (fundamental accounting equation).

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::statements::BalanceSheetRow;
use crate::repos::account_repo::AccountType;
use crate::repos::statement_repo::{self, StatementError};
use crate::services::normal_balance;

/// Balance sheet totals (Assets, Liabilities, Equity)
///
/// **Accounting Equation**: Assets = Liabilities + Equity
/// **Balance Guarantee**: is_balanced MUST be true or data is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceSheetTotals {
    /// Total assets in minor units (positive = debit balance)
    pub total_assets: i64,

    /// Total liabilities in minor units (positive = credit balance)
    pub total_liabilities: i64,

    /// Total equity in minor units (positive = credit balance)
    pub total_equity: i64,

    /// Whether the accounting equation is satisfied (assets == liabilities + equity)
    pub is_balanced: bool,
}

/// Balance sheet response with account balances and totals
///
/// **Currency Policy**: Single-currency only (currency is required parameter).
/// **Balance Guarantee**: totals.is_balanced MUST be true or data is invalid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceSheetResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String, // Required - no Optional wrapper
    pub rows: Vec<BalanceSheetRow>,
    pub totals: BalanceSheetTotals,
}

/// Errors that can occur during balance sheet operations
#[derive(Debug, Error)]
pub enum BalanceSheetError {
    #[error("Statement repository error: {0}")]
    StatementRepo(#[from] StatementError),

    #[error("Invalid tenant_id: {0}")]
    InvalidTenantId(String),

    #[error("Balance sheet does not balance: assets={assets}, liabilities={liabilities}, equity={equity}")]
    Unbalanced {
        assets: i64,
        liabilities: i64,
        equity: i64,
    },
}

/// Get balance sheet for a tenant and period
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
/// Balance sheet response with rows and totals
///
/// # Errors
/// Returns `Unbalanced` if assets != liabilities + equity (data integrity violation)
/// Returns `StatementRepo` error if period not found or database error
///
/// # Example
/// ```ignore
/// let bs = get_balance_sheet(
///     &pool,
///     "tenant_123",
///     period_id,
///     "USD",  // Required - no Optional wrapper
/// ).await?;
///
/// // Guaranteed: bs.totals.is_balanced == true (or error was returned)
/// assert!(bs.totals.is_balanced);
/// assert_eq!(bs.totals.total_assets, bs.totals.total_liabilities + bs.totals.total_equity);
/// ```
pub async fn get_balance_sheet(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    currency: &str,
) -> Result<BalanceSheetResponse, BalanceSheetError> {
    // Validate tenant_id
    if tenant_id.is_empty() {
        return Err(BalanceSheetError::InvalidTenantId(
            "tenant_id cannot be empty".to_string(),
        ));
    }

    // Query balance sheet from statement repository
    // Currency validation happens in statement_repo (ISO 4217 format check)
    let raw_rows = statement_repo::get_balance_sheet_rows(pool, tenant_id, period_id, currency)
        .await?;

    // Apply normal balance presentation logic to each row
    // This converts negative net_balance for liabilities/equity to positive presentation values
    let rows = apply_presentation_to_rows(raw_rows);

    // Calculate totals
    let totals = calculate_totals(&rows);

    // Validate balance (acceptance criteria: hard assertion test)
    // Fundamental accounting equation: Assets = Liabilities + Equity
    if !totals.is_balanced {
        return Err(BalanceSheetError::Unbalanced {
            assets: totals.total_assets,
            liabilities: totals.total_liabilities,
            equity: totals.total_equity,
        });
    }

    Ok(BalanceSheetResponse {
        tenant_id: tenant_id.to_string(),
        period_id,
        currency: currency.to_string(),
        rows,
        totals,
    })
}

/// Apply normal balance presentation logic to rows
///
/// Converts raw net_balance values to positive presentation values for display.
/// - Assets: Keep positive (debit normal)
/// - Liabilities: Convert negative to positive (credit normal)
/// - Equity: Convert negative to positive (credit normal)
fn apply_presentation_to_rows(rows: Vec<BalanceSheetRow>) -> Vec<BalanceSheetRow> {
    rows.into_iter()
        .map(|mut row| {
            // Parse account type string to enum
            let account_type = match row.account_type.as_str() {
                "asset" => AccountType::Asset,
                "liability" => AccountType::Liability,
                "equity" => AccountType::Equity,
                _ => return row, // Skip unknown types (defensive)
            };

            // Apply normal balance presentation logic
            row.amount_minor = normal_balance::apply_balance_sheet_presentation(
                &account_type,
                row.amount_minor,
            );

            row
        })
        .collect()
}

/// Calculate balance sheet totals
///
/// Sums assets, liabilities, and equity separately.
/// Per accounting principles: Assets MUST equal Liabilities + Equity.
///
/// **Note**: Rows must have presentation logic already applied (positive values).
fn calculate_totals(rows: &[BalanceSheetRow]) -> BalanceSheetTotals {
    let mut total_assets: i64 = 0;
    let mut total_liabilities: i64 = 0;
    let mut total_equity: i64 = 0;

    for row in rows {
        // Rows already have presentation applied, so amount_minor is positive
        match row.account_type.as_str() {
            "asset" => total_assets += row.amount_minor,
            "liability" => total_liabilities += row.amount_minor,
            "equity" => total_equity += row.amount_minor,
            _ => {
                // Should not happen due to SQL filter in statement_repo
                // Skip if unknown type (defensive programming)
            }
        }
    }

    // Check if fundamental accounting equation is satisfied
    let is_balanced = total_assets == (total_liabilities + total_equity);

    BalanceSheetTotals {
        total_assets,
        total_liabilities,
        total_equity,
        is_balanced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_totals_balanced() {
        let rows = vec![
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 100000, // $1,000.00
            },
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 30000, // $300.00
            },
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Retained Earnings".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 70000, // $700.00
            },
        ];

        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_assets, 100000);
        assert_eq!(totals.total_liabilities, 30000);
        assert_eq!(totals.total_equity, 70000);
        assert!(totals.is_balanced, "Assets (100000) should equal Liabilities (30000) + Equity (70000)");
        assert_eq!(totals.total_assets, totals.total_liabilities + totals.total_equity);
    }

    #[test]
    fn test_calculate_totals_unbalanced() {
        let rows = vec![
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 150000, // Assets too high
            },
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 30000,
            },
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Retained Earnings".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 70000,
            },
        ];

        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_assets, 150000);
        assert_eq!(totals.total_liabilities, 30000);
        assert_eq!(totals.total_equity, 70000);
        assert!(!totals.is_balanced, "Assets (150000) should NOT equal Liabilities (30000) + Equity (70000)");
    }

    #[test]
    fn test_calculate_totals_zero() {
        let rows: Vec<BalanceSheetRow> = vec![];
        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_assets, 0);
        assert_eq!(totals.total_liabilities, 0);
        assert_eq!(totals.total_equity, 0);
        assert!(totals.is_balanced, "Empty balance sheet should be balanced (0 = 0 + 0)");
    }

    #[test]
    fn test_calculate_totals_multiple_accounts_per_type() {
        let rows = vec![
            // Multiple asset accounts
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 50000,
            },
            BalanceSheetRow {
                account_code: "1100".to_string(),
                account_name: "Accounts Receivable".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 30000,
            },
            // Multiple liability accounts
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 20000,
            },
            BalanceSheetRow {
                account_code: "2100".to_string(),
                account_name: "Notes Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 10000,
            },
            // Multiple equity accounts
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Common Stock".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 40000,
            },
            BalanceSheetRow {
                account_code: "3100".to_string(),
                account_name: "Retained Earnings".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 10000,
            },
        ];

        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_assets, 80000); // 50000 + 30000
        assert_eq!(totals.total_liabilities, 30000); // 20000 + 10000
        assert_eq!(totals.total_equity, 50000); // 40000 + 10000
        assert!(totals.is_balanced);
        assert_eq!(totals.total_assets, totals.total_liabilities + totals.total_equity);
    }

    #[test]
    fn test_calculate_totals_deterministic_ordering() {
        // Verify totals calculation is order-independent
        let rows1 = vec![
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 100000,
            },
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 60000,
            },
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Equity".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 40000,
            },
        ];

        // Same data, different order
        let rows2 = vec![
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Equity".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 40000,
            },
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 100000,
            },
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 60000,
            },
        ];

        let totals1 = calculate_totals(&rows1);
        let totals2 = calculate_totals(&rows2);

        assert_eq!(totals1, totals2, "Totals should be deterministic regardless of row order");
    }

    #[test]
    fn test_calculate_totals_negative_balances() {
        // Test handling of negative balances (e.g., contra accounts)
        let rows = vec![
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: 100000,
            },
            BalanceSheetRow {
                account_code: "1900".to_string(),
                account_name: "Allowance for Doubtful Accounts".to_string(),
                account_type: "asset".to_string(),
                currency: "USD".to_string(),
                amount_minor: -10000, // Contra asset (negative)
            },
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "USD".to_string(),
                amount_minor: 30000,
            },
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Retained Earnings".to_string(),
                account_type: "equity".to_string(),
                currency: "USD".to_string(),
                amount_minor: 60000,
            },
        ];

        let totals = calculate_totals(&rows);
        assert_eq!(totals.total_assets, 90000); // 100000 - 10000
        assert_eq!(totals.total_liabilities, 30000);
        assert_eq!(totals.total_equity, 60000);
        assert!(totals.is_balanced);
    }

    #[test]
    fn test_balance_sheet_totals_serialization() {
        let totals = BalanceSheetTotals {
            total_assets: 100000,
            total_liabilities: 30000,
            total_equity: 70000,
            is_balanced: true,
        };

        let json = serde_json::to_string(&totals).unwrap();
        let deserialized: BalanceSheetTotals = serde_json::from_str(&json).unwrap();
        assert_eq!(totals, deserialized);
    }

    #[test]
    fn test_balance_sheet_response_serialization() {
        let response = BalanceSheetResponse {
            tenant_id: "tenant_123".to_string(),
            period_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            rows: vec![],
            totals: BalanceSheetTotals {
                total_assets: 0,
                total_liabilities: 0,
                total_equity: 0,
                is_balanced: true,
            },
        };

        let json = serde_json::to_string(&response).unwrap();
        let deserialized: BalanceSheetResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(response.tenant_id, deserialized.tenant_id);
        assert_eq!(response.period_id, deserialized.period_id);
        assert_eq!(response.currency, deserialized.currency);
    }

    #[test]
    fn test_balance_sheet_error_display() {
        let err = BalanceSheetError::InvalidTenantId("empty".to_string());
        assert!(err.to_string().contains("empty"));

        let err = BalanceSheetError::Unbalanced {
            assets: 100000,
            liabilities: 30000,
            equity: 60000,
        };
        assert!(err.to_string().contains("100000"));
        assert!(err.to_string().contains("30000"));
        assert!(err.to_string().contains("60000"));
    }
}

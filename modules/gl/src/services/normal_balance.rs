//! Normal Balance Presentation Logic (Phase 14.3)
//!
//! Provides correct debit/credit presentation based on account type.
//! Implements standard accounting conventions for normal balances.
//!
//! ## Normal Balance Rules
//!
//! - **Asset**: Debit normal → Increases with debits, decreases with credits
//! - **Expense**: Debit normal → Increases with debits, decreases with credits
//! - **Liability**: Credit normal → Increases with credits, decreases with debits
//! - **Equity**: Credit normal → Increases with credits, decreases with debits
//! - **Revenue**: Credit normal → Increases with credits, decreases with debits
//!
//! ## Presentation Conventions
//!
//! ### Trial Balance
//! Shows raw debit/credit totals - no sign adjustment needed.
//!
//! ### Income Statement (P&L)
//! - **Revenue**: Positive values increase profit (credit balance → negate to positive)
//! - **Expense**: Negative values decrease profit (debit balance → negate to negative)
//!
//! ### Balance Sheet
//! - **Asset**: Positive when debit balance (keep positive)
//! - **Liability**: Positive when credit balance (negate to positive)
//! - **Equity**: Positive when credit balance (negate to positive)

use crate::repos::account_repo::AccountType;

/// Apply normal balance presentation for Income Statement (P&L)
///
/// Converts raw net_balance to P&L presentation:
/// - Revenue (credit normal): negative net balance → positive amount (increases profit)
/// - Expense (debit normal): positive net balance → negative amount (decreases profit)
///
/// # Arguments
/// * `account_type` - The type of account
/// * `net_balance_minor` - Raw net balance (debit - credit) in minor units
///
/// # Returns
/// Signed amount for P&L presentation (positive = profit increase, negative = profit decrease)
///
/// # Example
/// ```ignore
/// // Revenue account with $100 credit balance (net_balance = -10000 cents)
/// let amount = apply_income_statement_presentation(&AccountType::Revenue, -10000);
/// assert_eq!(amount, 10000); // Positive in P&L
///
/// // Expense account with $50 debit balance (net_balance = 5000 cents)
/// let amount = apply_income_statement_presentation(&AccountType::Expense, 5000);
/// assert_eq!(amount, -5000); // Negative in P&L
/// ```
pub fn apply_income_statement_presentation(
    account_type: &AccountType,
    net_balance_minor: i64,
) -> i64 {
    match account_type {
        // Revenue: credit normal (net_balance < 0) → present as positive
        AccountType::Revenue => -net_balance_minor,

        // Expense: debit normal (net_balance > 0) → present as negative
        AccountType::Expense => -net_balance_minor,

        // Should not appear in income statement
        _ => net_balance_minor,
    }
}

/// Apply normal balance presentation for Balance Sheet
///
/// Converts raw net_balance to balance sheet presentation:
/// - Asset (debit normal): positive net balance → positive amount
/// - Liability (credit normal): negative net balance → positive amount
/// - Equity (credit normal): negative net balance → positive amount
///
/// # Arguments
/// * `account_type` - The type of account
/// * `net_balance_minor` - Raw net balance (debit - credit) in minor units
///
/// # Returns
/// Signed amount for balance sheet presentation (always positive for normal balances)
///
/// # Example
/// ```ignore
/// // Asset account with $200 debit balance (net_balance = 20000 cents)
/// let amount = apply_balance_sheet_presentation(&AccountType::Asset, 20000);
/// assert_eq!(amount, 20000); // Positive (debit normal)
///
/// // Liability account with $100 credit balance (net_balance = -10000 cents)
/// let amount = apply_balance_sheet_presentation(&AccountType::Liability, -10000);
/// assert_eq!(amount, 10000); // Positive (inverted from credit)
/// ```
pub fn apply_balance_sheet_presentation(
    account_type: &AccountType,
    net_balance_minor: i64,
) -> i64 {
    match account_type {
        // Asset: debit normal → keep sign (positive when debit balance)
        AccountType::Asset => net_balance_minor,

        // Liability: credit normal → invert sign (positive when credit balance)
        AccountType::Liability => -net_balance_minor,

        // Equity: credit normal → invert sign (positive when credit balance)
        AccountType::Equity => -net_balance_minor,

        // Should not appear in balance sheet
        _ => net_balance_minor,
    }
}

/// Determine if an account has a normal debit balance
///
/// Used for validation and presentation logic.
pub fn is_debit_normal(account_type: &AccountType) -> bool {
    matches!(account_type, AccountType::Asset | AccountType::Expense)
}

/// Determine if an account has a normal credit balance
///
/// Used for validation and presentation logic.
pub fn is_credit_normal(account_type: &AccountType) -> bool {
    matches!(
        account_type,
        AccountType::Liability | AccountType::Equity | AccountType::Revenue
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // INCOME STATEMENT PRESENTATION TESTS
    // ============================================================

    #[test]
    fn test_income_statement_revenue_credit_balance() {
        // Revenue with $100 credit balance (net_balance = -10000 cents)
        let amount = apply_income_statement_presentation(&AccountType::Revenue, -10000);
        assert_eq!(amount, 10000, "Revenue credit balance should be positive in P&L");
    }

    #[test]
    fn test_income_statement_revenue_debit_balance() {
        // Revenue with $50 debit balance (net_balance = 5000 cents) - unusual but valid
        let amount = apply_income_statement_presentation(&AccountType::Revenue, 5000);
        assert_eq!(amount, -5000, "Revenue debit balance should be negative in P&L");
    }

    #[test]
    fn test_income_statement_expense_debit_balance() {
        // Expense with $75 debit balance (net_balance = 7500 cents)
        let amount = apply_income_statement_presentation(&AccountType::Expense, 7500);
        assert_eq!(amount, -7500, "Expense debit balance should be negative in P&L");
    }

    #[test]
    fn test_income_statement_expense_credit_balance() {
        // Expense with $25 credit balance (net_balance = -2500 cents) - unusual but valid
        let amount = apply_income_statement_presentation(&AccountType::Expense, -2500);
        assert_eq!(amount, 2500, "Expense credit balance should be positive in P&L");
    }

    #[test]
    fn test_income_statement_zero_balance() {
        assert_eq!(
            apply_income_statement_presentation(&AccountType::Revenue, 0),
            0,
            "Zero revenue balance should remain zero"
        );
        assert_eq!(
            apply_income_statement_presentation(&AccountType::Expense, 0),
            0,
            "Zero expense balance should remain zero"
        );
    }

    // ============================================================
    // BALANCE SHEET PRESENTATION TESTS
    // ============================================================

    #[test]
    fn test_balance_sheet_asset_debit_balance() {
        // Asset with $200 debit balance (net_balance = 20000 cents)
        let amount = apply_balance_sheet_presentation(&AccountType::Asset, 20000);
        assert_eq!(amount, 20000, "Asset debit balance should be positive");
    }

    #[test]
    fn test_balance_sheet_asset_credit_balance() {
        // Asset with $50 credit balance (net_balance = -5000 cents) - unusual but valid
        let amount = apply_balance_sheet_presentation(&AccountType::Asset, -5000);
        assert_eq!(amount, -5000, "Asset credit balance should be negative");
    }

    #[test]
    fn test_balance_sheet_liability_credit_balance() {
        // Liability with $150 credit balance (net_balance = -15000 cents)
        let amount = apply_balance_sheet_presentation(&AccountType::Liability, -15000);
        assert_eq!(amount, 15000, "Liability credit balance should be positive");
    }

    #[test]
    fn test_balance_sheet_liability_debit_balance() {
        // Liability with $30 debit balance (net_balance = 3000 cents) - unusual but valid
        let amount = apply_balance_sheet_presentation(&AccountType::Liability, 3000);
        assert_eq!(amount, -3000, "Liability debit balance should be negative");
    }

    #[test]
    fn test_balance_sheet_equity_credit_balance() {
        // Equity with $500 credit balance (net_balance = -50000 cents)
        let amount = apply_balance_sheet_presentation(&AccountType::Equity, -50000);
        assert_eq!(amount, 50000, "Equity credit balance should be positive");
    }

    #[test]
    fn test_balance_sheet_equity_debit_balance() {
        // Equity with $100 debit balance (net_balance = 10000 cents) - unusual but valid
        let amount = apply_balance_sheet_presentation(&AccountType::Equity, 10000);
        assert_eq!(amount, -10000, "Equity debit balance should be negative");
    }

    #[test]
    fn test_balance_sheet_zero_balance() {
        assert_eq!(
            apply_balance_sheet_presentation(&AccountType::Asset, 0),
            0,
            "Zero asset balance should remain zero"
        );
        assert_eq!(
            apply_balance_sheet_presentation(&AccountType::Liability, 0),
            0,
            "Zero liability balance should remain zero"
        );
        assert_eq!(
            apply_balance_sheet_presentation(&AccountType::Equity, 0),
            0,
            "Zero equity balance should remain zero"
        );
    }

    // ============================================================
    // SIGN INVERSION TESTS (Acceptance Criteria)
    // ============================================================

    #[test]
    fn test_sign_inversion_revenue_vs_expense() {
        // Same absolute value, opposite account types
        let revenue_amount = apply_income_statement_presentation(&AccountType::Revenue, -10000);
        let expense_amount = apply_income_statement_presentation(&AccountType::Expense, 10000);

        // Both should have opposite signs but same absolute value
        assert_eq!(revenue_amount, 10000, "Revenue should be positive");
        assert_eq!(expense_amount, -10000, "Expense should be negative");
        assert_eq!(revenue_amount.abs(), expense_amount.abs(), "Absolute values should match");
    }

    #[test]
    fn test_sign_inversion_asset_vs_liability() {
        // Same absolute value, opposite account types
        let asset_amount = apply_balance_sheet_presentation(&AccountType::Asset, 20000);
        let liability_amount = apply_balance_sheet_presentation(&AccountType::Liability, -20000);

        // Both should be positive (normal balances)
        assert_eq!(asset_amount, 20000, "Asset debit balance should be positive");
        assert_eq!(liability_amount, 20000, "Liability credit balance should be positive");
    }

    #[test]
    fn test_sign_inversion_double_negation() {
        // Credit normal account with credit balance
        // net_balance = -10000 (credit balance)
        // Presentation: -(-10000) = 10000 (positive)
        let amount = apply_balance_sheet_presentation(&AccountType::Liability, -10000);
        assert_eq!(amount, 10000, "Double negation should yield positive");

        // Verify it's idempotent when applied correctly
        let net_balance = -10000i64;
        let presented = -net_balance;
        assert_eq!(presented, 10000);
    }

    // ============================================================
    // ZERO DRIFT TESTS (Acceptance Criteria)
    // ============================================================

    #[test]
    fn test_zero_drift_income_statement() {
        // Scenario: $100 revenue, $75 expense (net profit = $25)
        let revenue = apply_income_statement_presentation(&AccountType::Revenue, -10000);
        let expense = apply_income_statement_presentation(&AccountType::Expense, 7500);

        let net_profit = revenue + expense;
        assert_eq!(net_profit, 2500, "Net profit should be $25 (2500 cents)");

        // Verify: revenue - expense = profit
        assert_eq!(revenue - expense.abs(), 2500);
    }

    #[test]
    fn test_zero_drift_balance_sheet() {
        // Accounting equation: Assets = Liabilities + Equity
        // Scenario: $500 assets, $200 liabilities, $300 equity
        let assets = apply_balance_sheet_presentation(&AccountType::Asset, 50000);
        let liabilities = apply_balance_sheet_presentation(&AccountType::Liability, -20000);
        let equity = apply_balance_sheet_presentation(&AccountType::Equity, -30000);

        // Assets should equal Liabilities + Equity
        assert_eq!(assets, liabilities + equity, "Accounting equation must hold");
        assert_eq!(assets, 50000);
        assert_eq!(liabilities, 20000);
        assert_eq!(equity, 30000);
        assert_eq!(liabilities + equity, 50000);
    }

    #[test]
    fn test_zero_drift_multi_currency_income_statement() {
        // Revenue in multiple currencies
        let revenue_usd = apply_income_statement_presentation(&AccountType::Revenue, -10000); // $100
        let revenue_eur = apply_income_statement_presentation(&AccountType::Revenue, -5000);  // €50
        let revenue_gbp = apply_income_statement_presentation(&AccountType::Revenue, -7500);  // £75

        // Expenses in multiple currencies
        let expense_usd = apply_income_statement_presentation(&AccountType::Expense, 6000);   // $60
        let expense_eur = apply_income_statement_presentation(&AccountType::Expense, 3000);   // €30
        let expense_gbp = apply_income_statement_presentation(&AccountType::Expense, 4500);   // £45

        // Per-currency profit
        let profit_usd = revenue_usd + expense_usd;
        let profit_eur = revenue_eur + expense_eur;
        let profit_gbp = revenue_gbp + expense_gbp;

        assert_eq!(profit_usd, 4000, "USD profit should be $40");
        assert_eq!(profit_eur, 2000, "EUR profit should be €20");
        assert_eq!(profit_gbp, 3000, "GBP profit should be £30");

        // Each currency should maintain accounting integrity independently
        assert!(profit_usd > 0, "USD should be profitable");
        assert!(profit_eur > 0, "EUR should be profitable");
        assert!(profit_gbp > 0, "GBP should be profitable");
    }

    #[test]
    fn test_zero_drift_multi_currency_balance_sheet() {
        // USD balances
        let assets_usd = apply_balance_sheet_presentation(&AccountType::Asset, 50000);
        let liabilities_usd = apply_balance_sheet_presentation(&AccountType::Liability, -30000);
        let equity_usd = apply_balance_sheet_presentation(&AccountType::Equity, -20000);

        // EUR balances
        let assets_eur = apply_balance_sheet_presentation(&AccountType::Asset, 25000);
        let liabilities_eur = apply_balance_sheet_presentation(&AccountType::Liability, -15000);
        let equity_eur = apply_balance_sheet_presentation(&AccountType::Equity, -10000);

        // Each currency must balance independently
        assert_eq!(assets_usd, liabilities_usd + equity_usd, "USD must balance");
        assert_eq!(assets_eur, liabilities_eur + equity_eur, "EUR must balance");

        // Verify no drift in calculations
        assert_eq!(assets_usd, 50000);
        assert_eq!(liabilities_usd + equity_usd, 50000);
        assert_eq!(assets_eur, 25000);
        assert_eq!(liabilities_eur + equity_eur, 25000);
    }

    // ============================================================
    // NORMAL BALANCE CLASSIFICATION TESTS
    // ============================================================

    #[test]
    fn test_is_debit_normal() {
        assert!(is_debit_normal(&AccountType::Asset), "Asset should be debit normal");
        assert!(is_debit_normal(&AccountType::Expense), "Expense should be debit normal");
        assert!(!is_debit_normal(&AccountType::Liability), "Liability should not be debit normal");
        assert!(!is_debit_normal(&AccountType::Equity), "Equity should not be debit normal");
        assert!(!is_debit_normal(&AccountType::Revenue), "Revenue should not be debit normal");
    }

    #[test]
    fn test_is_credit_normal() {
        assert!(is_credit_normal(&AccountType::Liability), "Liability should be credit normal");
        assert!(is_credit_normal(&AccountType::Equity), "Equity should be credit normal");
        assert!(is_credit_normal(&AccountType::Revenue), "Revenue should be credit normal");
        assert!(!is_credit_normal(&AccountType::Asset), "Asset should not be credit normal");
        assert!(!is_credit_normal(&AccountType::Expense), "Expense should not be credit normal");
    }

    #[test]
    fn test_mutual_exclusivity_normal_balance() {
        // Every account type should be either debit normal or credit normal, not both
        for account_type in &[
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Equity,
            AccountType::Revenue,
            AccountType::Expense,
        ] {
            let is_debit = is_debit_normal(account_type);
            let is_credit = is_credit_normal(account_type);

            assert!(
                is_debit != is_credit,
                "{:?} must be exclusively debit or credit normal",
                account_type
            );
        }
    }
}

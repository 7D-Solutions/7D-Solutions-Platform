//! Statement Domain Models
//!
//! Canonical domain structs for financial statement outputs.
//! Pure domain models with no DB or formatting logic.
//! Used by statement services (trial balance, income statement, balance sheet).

use serde::{Deserialize, Serialize};

/// Trial Balance Row
///
/// Represents a single account row in a trial balance statement.
/// Contains account metadata and debit/credit balances for a specific period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialBalanceRow {
    /// Account code (e.g., "1000", "4000")
    pub account_code: String,

    /// Account name (e.g., "Cash", "Revenue")
    pub account_name: String,

    /// Account type (asset, liability, equity, revenue, expense)
    pub account_type: String,

    /// Normal balance direction (debit or credit)
    pub normal_balance: String,

    /// Currency code (ISO 4217, e.g., "USD", "EUR")
    pub currency: String,

    /// Total debits in minor units (cents)
    pub debit_total_minor: i64,

    /// Total credits in minor units (cents)
    pub credit_total_minor: i64,

    /// Net balance in minor units (debits - credits)
    pub net_balance_minor: i64,
}

/// Income Statement Row (P&L)
///
/// Represents a single account row in an income statement.
/// Contains revenue or expense account data for a specific period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomeStatementRow {
    /// Account code (e.g., "4000", "5000")
    pub account_code: String,

    /// Account name (e.g., "Sales Revenue", "Salaries Expense")
    pub account_name: String,

    /// Account type (revenue or expense only)
    pub account_type: String,

    /// Currency code (ISO 4217)
    pub currency: String,

    /// Account balance in minor units
    /// Positive for revenue (credit balance), negative for expense (debit balance)
    pub amount_minor: i64,
}

/// Balance Sheet Row
///
/// Represents a single account row in a balance sheet.
/// Contains asset, liability, or equity account data at a specific point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceSheetRow {
    /// Account code (e.g., "1000", "2000", "3000")
    pub account_code: String,

    /// Account name (e.g., "Cash", "Accounts Payable", "Retained Earnings")
    pub account_name: String,

    /// Account type (asset, liability, or equity only)
    pub account_type: String,

    /// Currency code (ISO 4217)
    pub currency: String,

    /// Account balance in minor units
    /// Positive for assets (debit balance), positive for liabilities/equity (credit balance)
    pub amount_minor: i64,
}

/// Statement Totals
///
/// Common totals structure used across all financial statements.
/// Provides aggregated debit/credit totals and balance verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementTotals {
    /// Total debits across all rows in minor units
    pub total_debits: i64,

    /// Total credits across all rows in minor units
    pub total_credits: i64,

    /// Whether debits equal credits (accounting equation satisfied)
    pub is_balanced: bool,
}

/// Per-Currency Totals
///
/// Aggregated totals for a specific currency in multi-currency statements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyTotals {
    /// Currency code (ISO 4217)
    pub currency: String,

    /// Total debits in this currency (minor units)
    pub total_debits: i64,

    /// Total credits in this currency (minor units)
    pub total_credits: i64,

    /// Whether this currency is balanced (debits = credits)
    pub is_balanced: bool,
}

/// Cash Flow Row
///
/// Represents a single account's cash flow activity classified into
/// operating, investing, or financing category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashFlowRow {
    /// Account code (e.g., "1000", "4000")
    pub account_code: String,

    /// Account name (e.g., "Cash", "Revenue")
    pub account_name: String,

    /// Cash flow category: operating, investing, or financing
    pub category: String,

    /// Currency code (ISO 4217)
    pub currency: String,

    /// Net cash flow amount in minor units (positive = inflow, negative = outflow)
    pub amount_minor: i64,
}

/// Cash Flow Category Totals
///
/// Aggregated totals for a single cash flow category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashFlowCategoryTotal {
    /// Category name: operating, investing, or financing
    pub category: String,

    /// Total net cash flow for this category in minor units
    pub total_minor: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trial_balance_row_serialization() {
        let row = TrialBalanceRow {
            account_code: "1000".to_string(),
            account_name: "Cash".to_string(),
            account_type: "asset".to_string(),
            normal_balance: "debit".to_string(),
            currency: "USD".to_string(),
            debit_total_minor: 100000,
            credit_total_minor: 0,
            net_balance_minor: 100000,
        };

        let json = serde_json::to_string(&row).unwrap();
        let deserialized: TrialBalanceRow = serde_json::from_str(&json).unwrap();
        assert_eq!(row, deserialized);
    }

    #[test]
    fn test_income_statement_row_serialization() {
        let row = IncomeStatementRow {
            account_code: "4000".to_string(),
            account_name: "Sales Revenue".to_string(),
            account_type: "revenue".to_string(),
            currency: "USD".to_string(),
            amount_minor: 500000,
        };

        let json = serde_json::to_string(&row).unwrap();
        let deserialized: IncomeStatementRow = serde_json::from_str(&json).unwrap();
        assert_eq!(row, deserialized);
    }

    #[test]
    fn test_balance_sheet_row_serialization() {
        let row = BalanceSheetRow {
            account_code: "1000".to_string(),
            account_name: "Cash".to_string(),
            account_type: "asset".to_string(),
            currency: "USD".to_string(),
            amount_minor: 250000,
        };

        let json = serde_json::to_string(&row).unwrap();
        let deserialized: BalanceSheetRow = serde_json::from_str(&json).unwrap();
        assert_eq!(row, deserialized);
    }

    #[test]
    fn test_statement_totals_serialization() {
        let totals = StatementTotals {
            total_debits: 100000,
            total_credits: 100000,
            is_balanced: true,
        };

        let json = serde_json::to_string(&totals).unwrap();
        let deserialized: StatementTotals = serde_json::from_str(&json).unwrap();
        assert_eq!(totals, deserialized);
    }

    #[test]
    fn test_currency_totals_serialization() {
        let totals = CurrencyTotals {
            currency: "USD".to_string(),
            total_debits: 100000,
            total_credits: 100000,
            is_balanced: true,
        };

        let json = serde_json::to_string(&totals).unwrap();
        let deserialized: CurrencyTotals = serde_json::from_str(&json).unwrap();
        assert_eq!(totals, deserialized);
    }

    #[test]
    fn test_statement_totals_balanced() {
        let totals = StatementTotals {
            total_debits: 100000,
            total_credits: 100000,
            is_balanced: true,
        };
        assert!(totals.is_balanced);
    }

    #[test]
    fn test_statement_totals_unbalanced() {
        let totals = StatementTotals {
            total_debits: 150000,
            total_credits: 100000,
            is_balanced: false,
        };
        assert!(!totals.is_balanced);
    }

    /// Comprehensive snapshot serialization test
    ///
    /// Verifies JSON structure stability for all statement domain models.
    /// This test serves as a contract test to ensure serialization format
    /// remains stable across refactorings.
    #[test]
    fn test_comprehensive_snapshot_serialization() {
        // Trial Balance Row
        let tb_row = TrialBalanceRow {
            account_code: "1000".to_string(),
            account_name: "Cash".to_string(),
            account_type: "asset".to_string(),
            normal_balance: "debit".to_string(),
            currency: "USD".to_string(),
            debit_total_minor: 100000,
            credit_total_minor: 25000,
            net_balance_minor: 75000,
        };

        let tb_json = serde_json::to_value(&tb_row).unwrap();
        assert_eq!(tb_json["account_code"], "1000");
        assert_eq!(tb_json["account_name"], "Cash");
        assert_eq!(tb_json["account_type"], "asset");
        assert_eq!(tb_json["normal_balance"], "debit");
        assert_eq!(tb_json["currency"], "USD");
        assert_eq!(tb_json["debit_total_minor"], 100000);
        assert_eq!(tb_json["credit_total_minor"], 25000);
        assert_eq!(tb_json["net_balance_minor"], 75000);

        // Income Statement Row
        let is_row = IncomeStatementRow {
            account_code: "4000".to_string(),
            account_name: "Sales Revenue".to_string(),
            account_type: "revenue".to_string(),
            currency: "USD".to_string(),
            amount_minor: 500000,
        };

        let is_json = serde_json::to_value(&is_row).unwrap();
        assert_eq!(is_json["account_code"], "4000");
        assert_eq!(is_json["account_name"], "Sales Revenue");
        assert_eq!(is_json["account_type"], "revenue");
        assert_eq!(is_json["currency"], "USD");
        assert_eq!(is_json["amount_minor"], 500000);

        // Balance Sheet Row
        let bs_row = BalanceSheetRow {
            account_code: "2000".to_string(),
            account_name: "Accounts Payable".to_string(),
            account_type: "liability".to_string(),
            currency: "USD".to_string(),
            amount_minor: 150000,
        };

        let bs_json = serde_json::to_value(&bs_row).unwrap();
        assert_eq!(bs_json["account_code"], "2000");
        assert_eq!(bs_json["account_name"], "Accounts Payable");
        assert_eq!(bs_json["account_type"], "liability");
        assert_eq!(bs_json["currency"], "USD");
        assert_eq!(bs_json["amount_minor"], 150000);

        // Statement Totals
        let totals = StatementTotals {
            total_debits: 100000,
            total_credits: 100000,
            is_balanced: true,
        };

        let totals_json = serde_json::to_value(&totals).unwrap();
        assert_eq!(totals_json["total_debits"], 100000);
        assert_eq!(totals_json["total_credits"], 100000);
        assert_eq!(totals_json["is_balanced"], true);

        // Currency Totals
        let currency_totals = CurrencyTotals {
            currency: "EUR".to_string(),
            total_debits: 50000,
            total_credits: 50000,
            is_balanced: true,
        };

        let ct_json = serde_json::to_value(&currency_totals).unwrap();
        assert_eq!(ct_json["currency"], "EUR");
        assert_eq!(ct_json["total_debits"], 50000);
        assert_eq!(ct_json["total_credits"], 50000);
        assert_eq!(ct_json["is_balanced"], true);

        // Verify round-trip serialization for all types
        let tb_roundtrip: TrialBalanceRow = serde_json::from_value(tb_json).unwrap();
        assert_eq!(tb_row, tb_roundtrip);

        let is_roundtrip: IncomeStatementRow = serde_json::from_value(is_json).unwrap();
        assert_eq!(is_row, is_roundtrip);

        let bs_roundtrip: BalanceSheetRow = serde_json::from_value(bs_json).unwrap();
        assert_eq!(bs_row, bs_roundtrip);

        let totals_roundtrip: StatementTotals = serde_json::from_value(totals_json).unwrap();
        assert_eq!(totals, totals_roundtrip);

        let ct_roundtrip: CurrencyTotals = serde_json::from_value(ct_json).unwrap();
        assert_eq!(currency_totals, ct_roundtrip);
    }
}

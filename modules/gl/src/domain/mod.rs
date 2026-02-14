//! Domain Models
//!
//! Pure domain structs with no DB or formatting logic.

pub mod statements;

pub use statements::{
    BalanceSheetRow, CurrencyTotals, IncomeStatementRow, StatementTotals, TrialBalanceRow,
};

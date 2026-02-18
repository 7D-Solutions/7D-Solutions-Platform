//! Bank reconciliation — matches imported statement lines to payment-event
//! transactions. Matches are append-only; rematches create a new row and set
//! `superseded_by` on the old one.

pub mod engine;
pub mod models;
pub mod service;

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ReconError {
    #[error("Statement line not found: {0}")]
    StatementLineNotFound(Uuid),

    #[error("Bank transaction not found: {0}")]
    TransactionNotFound(Uuid),

    #[error("Match not found: {0}")]
    MatchNotFound(Uuid),

    #[error("Amount mismatch: statement line {stmt_amount} != transaction {txn_amount}")]
    AmountMismatch { stmt_amount: i64, txn_amount: i64 },

    #[error("Currency mismatch: {stmt_currency} != {txn_currency}")]
    CurrencyMismatch {
        stmt_currency: String,
        txn_currency: String,
    },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

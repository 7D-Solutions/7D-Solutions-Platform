//! Reconciliation matching strategies.
//!
//! A `MatchStrategy` scores candidate pairs of statement lines vs transactions.
//! The engine calls `score()` for every (line, txn) pair; returning `None`
//! means the pair is not a valid candidate.

pub mod credit_card;

use rust_decimal::Decimal;

use super::models::UnmatchedTxn;

/// Strategy for scoring candidate matches.
///
/// Returns `None` when mandatory criteria fail (no match possible).
/// Returns `Some(confidence)` in `[0.5000, 1.0000]` for valid candidates.
pub trait MatchStrategy {
    fn score(&self, statement_line: &UnmatchedTxn, transaction: &UnmatchedTxn) -> Option<Decimal>;

    /// When true, the engine can bucket candidates by exact amount+currency
    /// before calling `score`, avoiding a full cross-product scan.
    fn requires_exact_amount_currency(&self) -> bool {
        true
    }
}

//! Currency Conversion Types (Phase 23a, bd-24y)
//!
//! Core type definitions for FX currency conversion: rate snapshots,
//! converted amounts, journal line results, and conversion errors.

use chrono::{DateTime, Utc};
use uuid::Uuid;

// ============================================================================
// Core types
// ============================================================================

/// A specific FX rate snapshot used for conversion.
///
/// This is the "receipt" that ties a converted amount to the exact rate used.
/// Every journal line that involves FX must carry this reference.
#[derive(Debug, Clone, PartialEq)]
pub struct RateSnapshot {
    /// UUID of the fx_rates row
    pub rate_id: Uuid,
    /// The exchange rate: 1 base = rate quote
    pub rate: f64,
    /// The inverse rate: 1 quote = inverse_rate base
    pub inverse_rate: f64,
    /// When this rate became effective
    pub effective_at: DateTime<Utc>,
    /// Base currency (ISO 4217)
    pub base_currency: String,
    /// Quote currency (ISO 4217)
    pub quote_currency: String,
}

/// Result of converting a single monetary amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedAmount {
    /// Original amount in transaction currency (minor units, e.g. cents)
    pub transaction_amount_minor: i64,
    /// Converted amount in reporting currency (minor units)
    pub reporting_amount_minor: i64,
}

/// A journal line pair (debit + credit) after currency conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedLine {
    /// Original debit in transaction currency (minor units)
    pub txn_debit_minor: i64,
    /// Original credit in transaction currency (minor units)
    pub txn_credit_minor: i64,
    /// Converted debit in reporting currency (minor units)
    pub rpt_debit_minor: i64,
    /// Converted credit in reporting currency (minor units)
    pub rpt_credit_minor: i64,
}

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur during currency conversion.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConversionError {
    #[error("Rate must be positive and finite, got {0}")]
    InvalidRate(String),

    #[error("Amount must be non-negative (minor units), got {0}")]
    NegativeAmount(i64),

    #[error(
        "Currency mismatch: rate is {rate_base}/{rate_quote} but conversion requested {from}/{to}"
    )]
    CurrencyMismatch {
        rate_base: String,
        rate_quote: String,
        from: String,
        to: String,
    },

    #[error("Balance invariant violated: total debits ({debits}) != total credits ({credits}) in reporting currency")]
    BalanceInvariant { debits: i64, credits: i64 },
}

//! Treasury transaction types.
//!
//! Transactions originate from two sources:
//! 1. Statement import (CSV upload → bank_statement_lines → bank_transactions)
//! 2. Event ingestion (Payments/AP events → normalized bank_transactions)
//!
//! All amounts stored as i64 minor units (e.g. cents). Positive = credit (money in),
//! negative = debit (money out).
//!
//! Credit card transactions carry optional auth_date, settle_date, merchant_name,
//! and merchant_category_code (MCC). These are NULL for bank transactions.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request to insert a single transaction from an event source.
#[derive(Debug, Clone)]
pub struct InsertBankTxnRequest {
    pub app_id: String,
    pub account_id: Uuid,
    /// Positive = credit (money in), negative = debit (money out)
    pub amount_minor: i64,
    pub currency: String,
    pub transaction_date: NaiveDate,
    pub description: Option<String>,
    pub reference: Option<String>,
    /// Stable dedup key — use event_id to guarantee exactly-once per event.
    pub external_id: String,
    // CC-specific (None for bank transactions)
    /// Date card was authorised (may differ from settle_date).
    pub auth_date: Option<NaiveDate>,
    /// Date transaction settled with the issuer.
    pub settle_date: Option<NaiveDate>,
    /// Cleaned merchant descriptor from statement.
    pub merchant_name: Option<String>,
    /// ISO 18245 MCC (4-digit string).
    pub merchant_category_code: Option<String>,
}

/// A normalized treasury transaction row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BankTransaction {
    pub id: Uuid,
    pub app_id: String,
    pub account_id: Uuid,
    pub statement_id: Option<Uuid>,
    pub transaction_date: NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub reference: Option<String>,
    pub external_id: Option<String>,
    // CC-specific fields (NULL for bank transactions)
    pub auth_date: Option<NaiveDate>,
    pub settle_date: Option<NaiveDate>,
    pub merchant_name: Option<String>,
    pub merchant_category_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

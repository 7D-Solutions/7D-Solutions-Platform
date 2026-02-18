//! Reconciliation domain models — match records and request/response types.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Enums (mirror SQL enums)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "treasury_recon_match_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ReconMatchStatus {
    Pending,
    Confirmed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "treasury_recon_match_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ReconMatchType {
    Auto,
    Manual,
    Suggested,
}

// ============================================================================
// DB row
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReconMatch {
    pub id: Uuid,
    pub app_id: String,
    pub statement_line_id: Option<Uuid>,
    pub bank_transaction_id: Uuid,
    pub gl_entry_id: Option<i64>,
    pub match_type: ReconMatchType,
    pub confidence_score: Option<rust_decimal::Decimal>,
    pub matched_by: Option<String>,
    pub status: ReconMatchStatus,
    pub superseded_by: Option<Uuid>,
    pub matched_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Lightweight view used by the auto-match engine.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UnmatchedTxn {
    pub id: Uuid,
    pub account_id: Uuid,
    pub transaction_date: NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub reference: Option<String>,
    pub statement_id: Option<Uuid>,
}

// ============================================================================
// Request / response types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct AutoMatchRequest {
    pub account_id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoMatchResult {
    pub matches_created: usize,
    pub unmatched_statement_lines: usize,
    pub unmatched_transactions: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManualMatchRequest {
    pub statement_line_id: Uuid,
    pub bank_transaction_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListMatchesQuery {
    pub account_id: Uuid,
    #[serde(default)]
    pub include_superseded: bool,
}

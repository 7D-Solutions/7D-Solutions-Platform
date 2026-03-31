//! Elimination journal generation and posting.
//!
//! Generates elimination suggestions from intercompany matches and
//! optionally posts them to GL with exactly-once semantics.

pub mod service;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// A suggested elimination journal entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EliminationSuggestion {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub rule_type: String,
    pub entity_a_tenant_id: String,
    pub entity_b_tenant_id: String,
    pub debit_account_code: String,
    pub credit_account_code: String,
    pub amount_minor: i64,
    pub description: String,
}

/// Result of posting elimination journals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EliminationPostResult {
    pub group_id: Uuid,
    pub period_id: Uuid,
    pub as_of: NaiveDate,
    pub posted_count: usize,
    pub idempotency_key: String,
    pub journal_entry_ids: Vec<Uuid>,
    pub posted_at: DateTime<Utc>,
    pub already_posted: bool,
}

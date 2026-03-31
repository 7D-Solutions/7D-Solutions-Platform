//! Intercompany transaction matching.
//!
//! Identifies and matches intercompany balances across entities in a
//! consolidation group. Matching is purely in-memory — no DB writes.

pub mod matching;
pub mod service;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// A matched intercompany balance pair across two entities.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IntercompanyMatch {
    pub rule_id: Uuid,
    pub rule_name: String,
    pub rule_type: String,
    pub entity_a_tenant_id: String,
    pub entity_b_tenant_id: String,
    pub debit_account_code: String,
    pub credit_account_code: String,
    /// Amount matched (minor units) — the minimum of both sides.
    pub match_amount_minor: i64,
    /// Unmatched debit-side remainder.
    pub debit_unmatched_minor: i64,
    /// Unmatched credit-side remainder.
    pub credit_unmatched_minor: i64,
}

/// Per-entity, per-account balance extracted from a trial balance.
#[derive(Debug, Clone)]
pub struct EntityAccountBalance {
    pub entity_tenant_id: String,
    pub account_code: String,
    pub account_name: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub net_minor: i64,
}

/// Result of intercompany matching for a group + period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntercompanyMatchResult {
    pub group_id: Uuid,
    pub as_of: NaiveDate,
    pub matches: Vec<IntercompanyMatch>,
    pub unmatched_count: usize,
    pub total_matched_minor: i64,
}

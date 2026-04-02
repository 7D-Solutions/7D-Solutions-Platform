//! Revenue Recognition (Revrec) event contracts for GL (Phase 24a)
//!
//! Defines the canonical data model and event contracts for ASC 606 / IFRS 15
//! revenue recognition:
//!
//! ## Entities
//! - `RevenueContract`        — a customer contract with bundled promises
//! - `PerformanceObligation`  — a distinct promise within a contract
//! - `RecognitionSchedule`    — amortization schedule: when to post recognition entries
//! - `ScheduleLine`           — one period's recognition entry in a schedule
//! - `RecognitionRun`         — a single recognition posting for one obligation/period
//! - `ContractModification`   — amendment to an existing contract
//! - `AllocationChange`       — reallocation of transaction price on modification
//!
//! ## Events
//! - `revrec.contract_created`   — DATA_MUTATION (contract + obligations locked)
//! - `revrec.schedule_created`   — DATA_MUTATION (recognition schedule computed)
//! - `revrec.recognition_posted` — DATA_MUTATION (revenue recognized for a period)
//! - `revrec.contract_modified`  — DATA_MUTATION (contract amended, schedule reallocated)
//!
//! ## Accounting semantics
//!
//! On contract creation the full transaction price is deferred:
//!   DR  Cash / Receivable
//!   CR  Deferred Revenue
//!
//! As each performance obligation is satisfied, revenue is recognized:
//!   DR  Deferred Revenue
//!   CR  Revenue
//!
//! Revrec is a ledger of promises. All entities are **append-only** and
//! **replay-safe**: idempotent event_ids prevent double-posting.
//!
//! ## Idempotency
//! All events carry a caller-supplied `event_id` derived from the entity's
//! stable business key. Duplicate event_ids are silently skipped by
//! `process_gl_posting_request` via the `processed_events` table.

pub mod contract_schedule;
pub mod recognition_amendment;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// Re-export everything from submodules for backwards compatibility
pub use contract_schedule::{
    build_contract_created_envelope, build_schedule_created_envelope, ContractCreatedPayload,
    ScheduleCreatedPayload,
};
pub use recognition_amendment::{
    build_contract_modified_envelope, build_recognition_posted_envelope, ContractModifiedPayload,
    RecognitionPostedPayload,
};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A revenue contract was created with one or more performance obligations
pub const EVENT_TYPE_CONTRACT_CREATED: &str = "revrec.contract_created";

/// A recognition schedule was computed for a performance obligation
pub const EVENT_TYPE_SCHEDULE_CREATED: &str = "revrec.schedule_created";

/// A period's revenue recognition was posted to GL (deferred → recognized)
pub const EVENT_TYPE_RECOGNITION_POSTED: &str = "revrec.recognition_posted";

/// A contract was modified (price change, obligation added/removed, term extension)
pub const EVENT_TYPE_CONTRACT_MODIFIED: &str = "revrec.contract_modified";

// ============================================================================
// Schema Version
// ============================================================================

/// Schema version for all revrec event payloads (v1)
pub const REVREC_SCHEMA_VERSION: &str = "1.0.0";

// ============================================================================
// Mutation Classes
// ============================================================================

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

// ============================================================================
// Recognition Pattern
// ============================================================================

/// How a performance obligation's revenue is recognized over time.
///
/// Aligns with ASC 606-10-25-27: a performance obligation is satisfied either
/// at a point in time or over time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecognitionPattern {
    /// Recognize ratably (straight-line) over the satisfaction period.
    ///
    /// Revenue per period = total_allocated / period_months.
    /// Example: 12-month SaaS license → 1/12 per month.
    RatableOverTime {
        /// Number of months over which to recognize (must be ≥ 1)
        period_months: u32,
    },

    /// Recognize 100% at a single point in time when the obligation is satisfied.
    ///
    /// Example: delivery of a custom report, one-time implementation.
    PointInTime,

    /// Recognize proportional to usage of a metric.
    ///
    /// Revenue per period = (period_usage / total_contracted_usage) * allocated_amount.
    /// Example: API call-based billing against a capacity commitment.
    UsageBased {
        /// The usage metric driving recognition (e.g. "api_calls", "gb_storage")
        metric: String,
        /// Total contracted usage quantity the transaction price was priced on
        total_contracted_quantity: f64,
        /// Unit of the metric (e.g. "calls", "GB")
        unit: String,
    },
}

// ============================================================================
// Performance Obligation
// ============================================================================

/// A distinct, identifiable promise to transfer goods or services to a customer.
///
/// Each obligation carries its own allocated transaction price and recognition
/// schedule. Obligations are embedded in the contract event and referenced by
/// schedule + recognition events.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PerformanceObligation {
    /// Stable business key for this obligation (idempotency anchor)
    pub obligation_id: Uuid,

    /// Human-readable name (e.g. "SaaS License", "Implementation Services")
    pub name: String,

    /// Detailed description of what is promised
    pub description: String,

    /// Transaction price allocated to this obligation (minor currency units)
    ///
    /// Sum of all obligation allocations must equal the contract's
    /// total_transaction_price_minor.
    pub allocated_amount_minor: i64,

    /// How and when revenue is recognized for this obligation
    pub recognition_pattern: RecognitionPattern,

    /// When satisfaction of this obligation begins (YYYY-MM-DD)
    pub satisfaction_start: String,

    /// When satisfaction ends (YYYY-MM-DD), None for open-ended or point-in-time
    pub satisfaction_end: Option<String>,
}

// ============================================================================
// Schedule Line
// ============================================================================

/// One period's recognition entry in a recognition schedule.
///
/// Represents the journal entry that should be posted in `period`:
///   DR  Deferred Revenue (`deferred_revenue_account`)
///   CR  Revenue          (`recognized_revenue_account`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleLine {
    /// Accounting period for this entry (YYYY-MM)
    pub period: String,

    /// Amount to recognize in this period (minor currency units, always positive)
    pub amount_to_recognize_minor: i64,

    /// Account to debit (usually "DEFERRED_REV" — reduces deferred balance)
    pub deferred_revenue_account: String,

    /// Account to credit (usually "REV" or a revenue sub-account)
    pub recognized_revenue_account: String,
}

// ============================================================================
// Allocation Change (for contract modifications)
// ============================================================================

/// A reallocation of transaction price to a performance obligation on amendment.
///
/// Used in `ContractModifiedPayload` to capture how the contract modification
/// redistributes value across obligations.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AllocationChange {
    /// The obligation whose allocation changed
    pub obligation_id: Uuid,

    /// Allocated amount before the modification (minor currency units)
    pub previous_allocated_minor: i64,

    /// Allocated amount after the modification (minor currency units)
    pub new_allocated_minor: i64,
}

// ============================================================================
// Contract Modification Type
// ============================================================================

/// Classification of a contract modification for audit and reporting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModificationType {
    /// Transaction price changed without adding/removing obligations
    PriceChange,

    /// Contract term extended (satisfaction_end pushed out)
    TermExtension,

    /// New performance obligation added to the contract
    ObligationAdded,

    /// Existing performance obligation removed from the contract
    ObligationRemoved,

    /// Multiple modification types in a single amendment
    Combined,
}

// ============================================================================
// Tests (shared types)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_constants_use_revrec_prefix() {
        assert!(EVENT_TYPE_CONTRACT_CREATED.starts_with("revrec."));
        assert!(EVENT_TYPE_SCHEDULE_CREATED.starts_with("revrec."));
        assert!(EVENT_TYPE_RECOGNITION_POSTED.starts_with("revrec."));
        assert!(EVENT_TYPE_CONTRACT_MODIFIED.starts_with("revrec."));
    }

    #[test]
    fn schema_version_is_stable() {
        assert_eq!(REVREC_SCHEMA_VERSION, "1.0.0");
    }

    // ─── RecognitionPattern ──────────────────────────────────────────────────

    #[test]
    fn recognition_pattern_ratable_serializes() {
        let pattern = RecognitionPattern::RatableOverTime { period_months: 12 };
        let json = serde_json::to_string(&pattern).expect("serialize");
        assert!(json.contains("ratable_over_time"));
        assert!(json.contains("period_months"));
        assert!(json.contains("12"));
    }

    #[test]
    fn recognition_pattern_point_in_time_serializes() {
        let pattern = RecognitionPattern::PointInTime;
        let json = serde_json::to_string(&pattern).expect("serialize");
        assert!(json.contains("point_in_time"));
    }

    #[test]
    fn recognition_pattern_usage_based_serializes() {
        let pattern = RecognitionPattern::UsageBased {
            metric: "api_calls".to_string(),
            total_contracted_quantity: 1_000_000.0,
            unit: "calls".to_string(),
        };
        let json = serde_json::to_string(&pattern).expect("serialize");
        assert!(json.contains("usage_based"));
        assert!(json.contains("api_calls"));
        assert!(json.contains("total_contracted_quantity"));
    }

    // ─── ModificationType ────────────────────────────────────────────────────

    #[test]
    fn modification_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ModificationType::PriceChange).expect("serialize"),
            "\"price_change\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::TermExtension).expect("serialize"),
            "\"term_extension\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::ObligationAdded).expect("serialize"),
            "\"obligation_added\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::ObligationRemoved).expect("serialize"),
            "\"obligation_removed\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::Combined).expect("serialize"),
            "\"combined\""
        );
    }

    #[test]
    fn all_event_types_are_revrec_prefixed() {
        let events = [
            EVENT_TYPE_CONTRACT_CREATED,
            EVENT_TYPE_SCHEDULE_CREATED,
            EVENT_TYPE_RECOGNITION_POSTED,
            EVENT_TYPE_CONTRACT_MODIFIED,
        ];
        for event in &events {
            assert!(
                event.starts_with("revrec."),
                "Event '{}' does not start with 'revrec.'",
                event
            );
        }
    }
}

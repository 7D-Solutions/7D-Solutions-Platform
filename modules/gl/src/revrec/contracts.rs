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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_gl_envelope, EventEnvelope};

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
// Payload: revrec.contract_created
// ============================================================================

/// Payload for revrec.contract_created
///
/// Emitted when a revenue contract is formally created with its performance
/// obligations. This is the root event of the revrec lifecycle.
///
/// Idempotency: caller MUST supply a deterministic event_id derived from contract_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractCreatedPayload {
    /// Stable business key for this contract (idempotency anchor)
    pub contract_id: Uuid,

    pub tenant_id: String,
    pub customer_id: String,

    /// Human-readable contract name / reference number
    pub contract_name: String,

    /// When the contract term begins (YYYY-MM-DD)
    pub contract_start: String,

    /// When the contract term ends (YYYY-MM-DD), None for open-ended contracts
    pub contract_end: Option<String>,

    /// Total transaction price (sum of all obligation allocations, minor currency units)
    pub total_transaction_price_minor: i64,

    /// ISO 4217 currency code (uppercase, e.g. "USD")
    pub currency: String,

    /// All performance obligations in this contract.
    ///
    /// Invariant: sum(obligation.allocated_amount_minor) == total_transaction_price_minor
    pub performance_obligations: Vec<PerformanceObligation>,

    /// Optional reference to the source CRM/billing contract ID
    pub external_contract_ref: Option<String>,

    pub created_at: DateTime<Utc>,
}

/// Build an envelope for revrec.contract_created
///
/// mutation_class: DATA_MUTATION (creates a new contract with locked obligations)
pub fn build_contract_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContractCreatedPayload,
) -> EventEnvelope<ContractCreatedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CONTRACT_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(REVREC_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: revrec.schedule_created
// ============================================================================

/// Payload for revrec.schedule_created
///
/// Emitted when a recognition schedule is computed for a performance obligation.
/// The schedule defines the amortization table: which periods to post and how much.
///
/// Idempotency: caller MUST supply a deterministic event_id derived from schedule_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleCreatedPayload {
    /// Stable business key for this schedule (idempotency anchor)
    pub schedule_id: Uuid,

    /// Contract this schedule belongs to
    pub contract_id: Uuid,

    /// Obligation this schedule amortizes
    pub obligation_id: Uuid,

    pub tenant_id: String,

    /// Total amount to recognize across all periods (minor currency units)
    ///
    /// Must equal the obligation's allocated_amount_minor.
    pub total_to_recognize_minor: i64,

    /// ISO 4217 currency code (uppercase, e.g. "USD")
    pub currency: String,

    /// Ordered amortization entries — one per period
    pub lines: Vec<ScheduleLine>,

    /// First period that has a line (YYYY-MM)
    pub first_period: String,

    /// Last period that has a line (YYYY-MM)
    pub last_period: String,

    pub created_at: DateTime<Utc>,
}

/// Build an envelope for revrec.schedule_created
///
/// mutation_class: DATA_MUTATION (creates a new recognition schedule)
pub fn build_schedule_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ScheduleCreatedPayload,
) -> EventEnvelope<ScheduleCreatedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_SCHEDULE_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(REVREC_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: revrec.recognition_posted
// ============================================================================

/// Payload for revrec.recognition_posted
///
/// Emitted when a period's revenue recognition is posted to GL.
/// Creates a balanced journal entry:
///   DR  Deferred Revenue
///   CR  Revenue
///
/// Idempotency: caller MUST supply a deterministic event_id derived from
/// (schedule_id, period) or a stable run_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognitionPostedPayload {
    /// Stable business key for this recognition run (idempotency anchor)
    ///
    /// Deterministically derived from (schedule_id, period) to prevent
    /// double-recognition in the same period.
    pub run_id: Uuid,

    /// Contract this recognition belongs to
    pub contract_id: Uuid,

    /// Obligation whose revenue is being recognized
    pub obligation_id: Uuid,

    /// Schedule driving this recognition
    pub schedule_id: Uuid,

    pub tenant_id: String,

    /// Accounting period being recognized (YYYY-MM)
    pub period: String,

    /// Accounting date for the GL journal entry (YYYY-MM-DD)
    pub posting_date: String,

    /// Amount recognized in this run (minor currency units, always positive)
    pub amount_recognized_minor: i64,

    /// ISO 4217 currency code (uppercase, e.g. "USD")
    pub currency: String,

    /// Account debited: deferred revenue account (e.g. "DEFERRED_REV")
    pub deferred_revenue_account: String,

    /// Account credited: recognized revenue account (e.g. "REV")
    pub recognized_revenue_account: String,

    /// GL journal entry created for this posting (populated after posting succeeds)
    pub journal_entry_id: Option<Uuid>,

    /// Cumulative amount recognized for this obligation through this period
    pub cumulative_recognized_minor: i64,

    /// Remaining amount deferred for this obligation after this posting
    pub remaining_deferred_minor: i64,

    pub recognized_at: DateTime<Utc>,
}

/// Build an envelope for revrec.recognition_posted
///
/// mutation_class: DATA_MUTATION (creates a revenue recognition journal entry)
pub fn build_recognition_posted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: RecognitionPostedPayload,
) -> EventEnvelope<RecognitionPostedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECOGNITION_POSTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(REVREC_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: revrec.contract_modified
// ============================================================================

/// Payload for revrec.contract_modified
///
/// Emitted when a contract is amended. Modifications may:
/// - Change the transaction price (prospective or cumulative catch-up)
/// - Add new performance obligations
/// - Remove existing obligations
/// - Extend the contract term
///
/// Downstream schedules must be recomputed after a modification event.
/// Modification is append-only: the original contract event is never changed.
///
/// Idempotency: caller MUST supply a deterministic event_id derived from modification_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractModifiedPayload {
    /// Stable business key for this modification (idempotency anchor)
    pub modification_id: Uuid,

    /// The contract being amended
    pub contract_id: Uuid,

    pub tenant_id: String,

    /// Classification of the modification for audit and reporting
    pub modification_type: ModificationType,

    /// Date the modification takes effect (YYYY-MM-DD)
    pub effective_date: String,

    /// New total transaction price after modification, if changed (minor currency units)
    pub new_transaction_price_minor: Option<i64>,

    /// New performance obligations added by this modification
    pub added_obligations: Vec<PerformanceObligation>,

    /// IDs of obligations removed by this modification
    pub removed_obligation_ids: Vec<Uuid>,

    /// How the modification reallocated the transaction price across obligations
    pub reallocated_amounts: Vec<AllocationChange>,

    /// Human-readable reason for the modification
    /// (e.g. "contract_renewal", "scope_change", "price_adjustment")
    pub reason: String,

    /// Whether a cumulative catch-up adjustment is required.
    ///
    /// True when the modification is treated as a change to the existing contract
    /// (ASC 606-10-25-13(b)): recognized amount is adjusted in the current period.
    /// False when the modification is a separate new contract.
    pub requires_cumulative_catchup: bool,

    pub modified_at: DateTime<Utc>,
}

/// Build an envelope for revrec.contract_modified
///
/// mutation_class: DATA_MUTATION (records a contract amendment; schedules recomputed downstream)
pub fn build_contract_modified_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContractModifiedPayload,
) -> EventEnvelope<ContractModifiedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CONTRACT_MODIFIED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(REVREC_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn sample_obligation(allocated_minor: i64) -> PerformanceObligation {
        PerformanceObligation {
            obligation_id: Uuid::new_v4(),
            name: "SaaS License".to_string(),
            description: "12-month access to the platform".to_string(),
            allocated_amount_minor: allocated_minor,
            recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
            satisfaction_start: "2026-01-01".to_string(),
            satisfaction_end: Some("2026-12-31".to_string()),
        }
    }

    fn sample_contract_created() -> ContractCreatedPayload {
        let obligation = sample_obligation(120000_00); // $120,000
        ContractCreatedPayload {
            contract_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            contract_name: "Enterprise SaaS — Acme Corp 2026".to_string(),
            contract_start: "2026-01-01".to_string(),
            contract_end: Some("2026-12-31".to_string()),
            total_transaction_price_minor: 120000_00,
            currency: "USD".to_string(),
            performance_obligations: vec![obligation],
            external_contract_ref: Some("CRM-12345".to_string()),
            created_at: Utc::now(),
        }
    }

    fn sample_schedule_lines() -> Vec<ScheduleLine> {
        (1..=12)
            .map(|m| ScheduleLine {
                period: format!("2026-{:02}", m),
                amount_to_recognize_minor: 10000_00, // $10,000/month
                deferred_revenue_account: "DEFERRED_REV".to_string(),
                recognized_revenue_account: "REV".to_string(),
            })
            .collect()
    }

    fn sample_schedule_created(contract_id: Uuid, obligation_id: Uuid) -> ScheduleCreatedPayload {
        ScheduleCreatedPayload {
            schedule_id: Uuid::new_v4(),
            contract_id,
            obligation_id,
            tenant_id: "tenant-1".to_string(),
            total_to_recognize_minor: 120000_00,
            currency: "USD".to_string(),
            lines: sample_schedule_lines(),
            first_period: "2026-01".to_string(),
            last_period: "2026-12".to_string(),
            created_at: Utc::now(),
        }
    }

    fn sample_recognition_posted(
        contract_id: Uuid,
        obligation_id: Uuid,
        schedule_id: Uuid,
    ) -> RecognitionPostedPayload {
        RecognitionPostedPayload {
            run_id: Uuid::new_v4(),
            contract_id,
            obligation_id,
            schedule_id,
            tenant_id: "tenant-1".to_string(),
            period: "2026-01".to_string(),
            posting_date: "2026-01-31".to_string(),
            amount_recognized_minor: 10000_00,
            currency: "USD".to_string(),
            deferred_revenue_account: "DEFERRED_REV".to_string(),
            recognized_revenue_account: "REV".to_string(),
            journal_entry_id: Some(Uuid::new_v4()),
            cumulative_recognized_minor: 10000_00,
            remaining_deferred_minor: 110000_00,
            recognized_at: Utc::now(),
        }
    }

    // ─── Event type constants ────────────────────────────────────────────────

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

    // ─── revrec.contract_created ─────────────────────────────────────────────

    #[test]
    fn contract_created_envelope_has_correct_metadata() {
        let envelope = build_contract_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            sample_contract_created(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CONTRACT_CREATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, REVREC_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn contract_created_carries_causation_id() {
        let envelope = build_contract_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("crm-event-xyz".to_string()),
            sample_contract_created(),
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("crm-event-xyz"));
    }

    #[test]
    fn contract_created_payload_serializes_correctly() {
        let payload = sample_contract_created();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("contract_id"));
        assert!(json.contains("performance_obligations"));
        assert!(json.contains("recognition_pattern"));
        assert!(json.contains("ratable_over_time"));
        assert!(json.contains("total_transaction_price_minor"));
    }

    #[test]
    fn contract_obligations_allocation_sum_invariant() {
        // Invariant: sum of obligation amounts == total_transaction_price_minor
        let payload = sample_contract_created();
        let sum: i64 = payload
            .performance_obligations
            .iter()
            .map(|o| o.allocated_amount_minor)
            .sum();
        assert_eq!(sum, payload.total_transaction_price_minor);
    }

    // ─── revrec.schedule_created ─────────────────────────────────────────────

    #[test]
    fn schedule_created_envelope_has_correct_metadata() {
        let contract_id = Uuid::new_v4();
        let obligation_id = Uuid::new_v4();
        let envelope = build_schedule_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("contract-created-event".to_string()),
            sample_schedule_created(contract_id, obligation_id),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_SCHEDULE_CREATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, REVREC_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn schedule_lines_sum_equals_total() {
        let contract_id = Uuid::new_v4();
        let obligation_id = Uuid::new_v4();
        let payload = sample_schedule_created(contract_id, obligation_id);
        let lines_sum: i64 = payload.lines.iter().map(|l| l.amount_to_recognize_minor).sum();
        assert_eq!(lines_sum, payload.total_to_recognize_minor);
    }

    #[test]
    fn schedule_has_12_monthly_lines() {
        let payload = sample_schedule_created(Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(payload.lines.len(), 12);
        assert_eq!(payload.first_period, "2026-01");
        assert_eq!(payload.last_period, "2026-12");
    }

    #[test]
    fn schedule_created_payload_serializes_correctly() {
        let payload = sample_schedule_created(Uuid::new_v4(), Uuid::new_v4());
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("schedule_id"));
        assert!(json.contains("lines"));
        assert!(json.contains("deferred_revenue_account"));
        assert!(json.contains("DEFERRED_REV"));
    }

    // ─── revrec.recognition_posted ───────────────────────────────────────────

    #[test]
    fn recognition_posted_envelope_has_correct_metadata() {
        let contract_id = Uuid::new_v4();
        let obligation_id = Uuid::new_v4();
        let schedule_id = Uuid::new_v4();
        let envelope = build_recognition_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-3".to_string(),
            Some("schedule-created-event".to_string()),
            sample_recognition_posted(contract_id, obligation_id, schedule_id),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECOGNITION_POSTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, REVREC_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn recognition_posted_cumulative_plus_remaining_equals_total() {
        let payload = sample_recognition_posted(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        // After month 1: 10,000 recognized + 110,000 remaining = 120,000 total
        assert_eq!(
            payload.cumulative_recognized_minor + payload.remaining_deferred_minor,
            120000_00
        );
    }

    #[test]
    fn recognition_posted_payload_serializes_correctly() {
        let payload = sample_recognition_posted(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("run_id"));
        assert!(json.contains("period"));
        assert!(json.contains("amount_recognized_minor"));
        assert!(json.contains("cumulative_recognized_minor"));
        assert!(json.contains("remaining_deferred_minor"));
    }

    // ─── revrec.contract_modified ────────────────────────────────────────────

    #[test]
    fn contract_modified_envelope_has_correct_metadata() {
        let contract_id = Uuid::new_v4();
        let obligation_id = Uuid::new_v4();
        let payload = ContractModifiedPayload {
            modification_id: Uuid::new_v4(),
            contract_id,
            tenant_id: "tenant-1".to_string(),
            modification_type: ModificationType::PriceChange,
            effective_date: "2026-07-01".to_string(),
            new_transaction_price_minor: Some(150000_00),
            added_obligations: vec![],
            removed_obligation_ids: vec![],
            reallocated_amounts: vec![AllocationChange {
                obligation_id,
                previous_allocated_minor: 120000_00,
                new_allocated_minor: 150000_00,
            }],
            reason: "annual_price_increase".to_string(),
            requires_cumulative_catchup: true,
            modified_at: Utc::now(),
        };
        let envelope = build_contract_modified_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-4".to_string(),
            Some("crm-amendment-event".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CONTRACT_MODIFIED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, REVREC_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn contract_modified_payload_serializes_correctly() {
        let payload = ContractModifiedPayload {
            modification_id: Uuid::new_v4(),
            contract_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            modification_type: ModificationType::ObligationAdded,
            effective_date: "2026-04-01".to_string(),
            new_transaction_price_minor: Some(144000_00),
            added_obligations: vec![sample_obligation(24000_00)],
            removed_obligation_ids: vec![],
            reallocated_amounts: vec![],
            reason: "added_implementation_services".to_string(),
            requires_cumulative_catchup: false,
            modified_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("modification_id"));
        assert!(json.contains("modification_type"));
        assert!(json.contains("obligation_added"));
        assert!(json.contains("added_obligations"));
        assert!(json.contains("requires_cumulative_catchup"));
    }

    // ─── RecognitionPattern ──────────────────────────────────────────────────

    #[test]
    fn recognition_pattern_ratable_serializes() {
        let pattern = RecognitionPattern::RatableOverTime { period_months: 12 };
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("ratable_over_time"));
        assert!(json.contains("period_months"));
        assert!(json.contains("12"));
    }

    #[test]
    fn recognition_pattern_point_in_time_serializes() {
        let pattern = RecognitionPattern::PointInTime;
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("point_in_time"));
    }

    #[test]
    fn recognition_pattern_usage_based_serializes() {
        let pattern = RecognitionPattern::UsageBased {
            metric: "api_calls".to_string(),
            total_contracted_quantity: 1_000_000.0,
            unit: "calls".to_string(),
        };
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("usage_based"));
        assert!(json.contains("api_calls"));
        assert!(json.contains("total_contracted_quantity"));
    }

    // ─── ModificationType ────────────────────────────────────────────────────

    #[test]
    fn modification_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ModificationType::PriceChange).unwrap(),
            "\"price_change\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::TermExtension).unwrap(),
            "\"term_extension\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::ObligationAdded).unwrap(),
            "\"obligation_added\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::ObligationRemoved).unwrap(),
            "\"obligation_removed\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::Combined).unwrap(),
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

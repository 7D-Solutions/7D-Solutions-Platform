//! Recognition posting and contract modification event payloads + builders.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::events::envelope::{create_gl_envelope, EventEnvelope};

use super::{
    AllocationChange, ModificationType, PerformanceObligation, EVENT_TYPE_CONTRACT_MODIFIED,
    EVENT_TYPE_RECOGNITION_POSTED, MUTATION_CLASS_DATA_MUTATION, REVREC_SCHEMA_VERSION,
};

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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    use crate::revrec::contracts::RecognitionPattern;
    use chrono::Utc;

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
        let payload = sample_recognition_posted(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
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
}

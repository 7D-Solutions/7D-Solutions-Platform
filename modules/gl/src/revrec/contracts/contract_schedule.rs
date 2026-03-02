//! Contract creation and schedule creation event payloads + builders.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_gl_envelope, EventEnvelope};

use super::{
    PerformanceObligation, ScheduleLine, EVENT_TYPE_CONTRACT_CREATED, EVENT_TYPE_SCHEDULE_CREATED,
    MUTATION_CLASS_DATA_MUTATION, REVREC_SCHEMA_VERSION,
};

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
        let lines_sum: i64 = payload
            .lines
            .iter()
            .map(|l| l.amount_to_recognize_minor)
            .sum();
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
}

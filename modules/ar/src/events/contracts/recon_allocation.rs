//! Reconciliation and payment allocation event contracts:
//! ar.recon_run_started, ar.recon_match_applied, ar.recon_exception_raised,
//! ar.payment_allocated

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_ar_envelope, EventEnvelope};
use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A reconciliation run was initiated
pub const EVENT_TYPE_RECON_RUN_STARTED: &str = "ar.recon_run_started";

/// A reconciliation match was successfully applied (payment ↔ invoice)
pub const EVENT_TYPE_RECON_MATCH_APPLIED: &str = "ar.recon_match_applied";

/// A reconciliation exception was raised (unmatched or ambiguous)
pub const EVENT_TYPE_RECON_EXCEPTION_RAISED: &str = "ar.recon_exception_raised";

/// A payment was allocated to one or more invoices
pub const EVENT_TYPE_PAYMENT_ALLOCATED: &str = "ar.payment_allocated";

// ============================================================================
// Payload: ar.recon_run_started
// ============================================================================

/// Payload for ar.recon_run_started
///
/// Emitted when a reconciliation run is initiated. Anchors causation chain
/// for all match/exception events that follow in the same run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconRunStartedPayload {
    pub tenant_id: String,
    /// Stable ID for this reconciliation run (idempotency anchor)
    pub recon_run_id: Uuid,
    /// Number of payments included in this run
    pub payment_count: i32,
    /// Number of invoices considered for matching
    pub invoice_count: i32,
    /// Strategy used for matching (e.g. "fifo", "exact_match", "best_fit")
    pub matching_strategy: String,
    pub started_at: DateTime<Utc>,
}

/// Build an envelope for ar.recon_run_started
///
/// mutation_class: DATA_MUTATION (creates a reconciliation run record)
pub fn build_recon_run_started_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReconRunStartedPayload,
) -> EventEnvelope<ReconRunStartedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECON_RUN_STARTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.recon_match_applied
// ============================================================================

/// Payload for ar.recon_match_applied
///
/// Emitted when a reconciliation match is confirmed and applied.
/// A single payment may generate multiple match events (partial allocations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconMatchAppliedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    pub payment_id: String,
    pub invoice_id: String,
    /// Amount applied in this match (minor currency units)
    pub matched_amount_minor: i64,
    pub currency: String,
    /// Confidence score 0.0–1.0 (1.0 = exact match)
    pub confidence_score: f64,
    /// Match method applied (e.g. "exact", "fifo", "reference")
    pub match_method: String,
    pub matched_at: DateTime<Utc>,
}

/// Build an envelope for ar.recon_match_applied
///
/// mutation_class: DATA_MUTATION (creates a reconciliation match record)
pub fn build_recon_match_applied_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReconMatchAppliedPayload,
) -> EventEnvelope<ReconMatchAppliedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECON_MATCH_APPLIED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.recon_exception_raised
// ============================================================================

/// Classification of reconciliation exceptions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconExceptionKind {
    /// Payment received but no matching invoice found
    UnmatchedPayment,
    /// Invoice overdue but no corresponding payment
    UnmatchedInvoice,
    /// Payment amount does not match any invoice amount
    AmountMismatch,
    /// Multiple invoices match with equal confidence
    AmbiguousMatch,
    /// Duplicate payment reference detected
    DuplicateReference,
}

/// Payload for ar.recon_exception_raised
///
/// Emitted when a reconciliation exception cannot be automatically resolved.
/// Requires manual review or escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconExceptionRaisedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    /// Payment ID if the exception is payment-related (optional for invoice-only exceptions)
    pub payment_id: Option<String>,
    /// Invoice ID if the exception is invoice-related
    pub invoice_id: Option<String>,
    pub exception_kind: ReconExceptionKind,
    /// Human-readable description of the exception
    pub description: String,
    /// Amount involved in minor currency units (if applicable)
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub raised_at: DateTime<Utc>,
}

/// Build an envelope for ar.recon_exception_raised
///
/// mutation_class: DATA_MUTATION (creates an exception record for manual review)
pub fn build_recon_exception_raised_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReconExceptionRaisedPayload,
) -> EventEnvelope<ReconExceptionRaisedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECON_EXCEPTION_RAISED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.payment_allocated
// ============================================================================

/// A single allocation line: one invoice portion of the payment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationLine {
    pub invoice_id: String,
    /// Amount allocated to this invoice (minor currency units)
    pub allocated_minor: i64,
    /// Remaining balance on this invoice after allocation (minor currency units)
    pub remaining_after_minor: i64,
}

/// Payload for ar.payment_allocated
///
/// Emitted when a payment is allocated (partially or fully) to one or more invoices.
/// Uses FIFO by default; allocation strategy is included for auditability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAllocatedPayload {
    pub tenant_id: String,
    pub payment_id: String,
    pub customer_id: String,
    /// Total payment amount (minor currency units)
    pub payment_amount_minor: i64,
    /// Amount allocated in this event (may be partial)
    pub allocated_amount_minor: i64,
    /// Amount remaining unallocated after this event
    pub unallocated_amount_minor: i64,
    pub currency: String,
    /// Allocation strategy applied (e.g. "fifo", "specific", "proportional")
    pub allocation_strategy: String,
    /// Invoice allocations in this event (ordered by application)
    pub allocations: Vec<AllocationLine>,
    pub allocated_at: DateTime<Utc>,
}

/// Build an envelope for ar.payment_allocated
///
/// mutation_class: DATA_MUTATION (creates allocation records against invoices)
pub fn build_payment_allocated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PaymentAllocatedPayload,
) -> EventEnvelope<PaymentAllocatedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PAYMENT_ALLOCATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn recon_run_started_envelope_has_data_mutation_class() {
        let run_id = Uuid::new_v4();
        let payload = ReconRunStartedPayload {
            tenant_id: "tenant-1".to_string(),
            recon_run_id: run_id,
            payment_count: 10,
            invoice_count: 25,
            matching_strategy: "fifo".to_string(),
            started_at: Utc::now(),
        };
        let envelope = build_recon_run_started_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECON_RUN_STARTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn recon_match_applied_envelope_has_correct_metadata() {
        let payload = ReconMatchAppliedPayload {
            tenant_id: "tenant-1".to_string(),
            recon_run_id: Uuid::new_v4(),
            payment_id: "pay-55".to_string(),
            invoice_id: "inv-55".to_string(),
            matched_amount_minor: 10000,
            currency: "usd".to_string(),
            confidence_score: 1.0,
            match_method: "exact".to_string(),
            matched_at: Utc::now(),
        };
        let envelope = build_recon_match_applied_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECON_MATCH_APPLIED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
    }

    #[test]
    fn recon_exception_raised_envelope_has_correct_metadata() {
        let payload = ReconExceptionRaisedPayload {
            tenant_id: "tenant-1".to_string(),
            recon_run_id: Uuid::new_v4(),
            payment_id: Some("pay-99".to_string()),
            invoice_id: None,
            exception_kind: ReconExceptionKind::UnmatchedPayment,
            description: "No invoice found for payment reference PAY-99".to_string(),
            amount_minor: Some(20000),
            currency: Some("usd".to_string()),
            raised_at: Utc::now(),
        };
        let envelope = build_recon_exception_raised_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECON_EXCEPTION_RAISED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn payment_allocated_envelope_has_correct_metadata() {
        let payload = PaymentAllocatedPayload {
            tenant_id: "tenant-1".to_string(),
            payment_id: "pay-42".to_string(),
            customer_id: "cust-1".to_string(),
            payment_amount_minor: 30000,
            allocated_amount_minor: 25000,
            unallocated_amount_minor: 5000,
            currency: "usd".to_string(),
            allocation_strategy: "fifo".to_string(),
            allocations: vec![
                AllocationLine {
                    invoice_id: "inv-1".to_string(),
                    allocated_minor: 15000,
                    remaining_after_minor: 0,
                },
                AllocationLine {
                    invoice_id: "inv-2".to_string(),
                    allocated_minor: 10000,
                    remaining_after_minor: 5000,
                },
            ],
            allocated_at: Utc::now(),
        };
        let envelope = build_payment_allocated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_PAYMENT_ALLOCATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn recon_allocation_event_type_constants_use_ar_prefix() {
        assert!(EVENT_TYPE_RECON_RUN_STARTED.starts_with("ar."));
        assert!(EVENT_TYPE_RECON_MATCH_APPLIED.starts_with("ar."));
        assert!(EVENT_TYPE_RECON_EXCEPTION_RAISED.starts_with("ar."));
        assert!(EVENT_TYPE_PAYMENT_ALLOCATED.starts_with("ar."));
    }

    #[test]
    fn recon_exception_kind_serializes_as_snake_case() {
        let kind = ReconExceptionKind::UnmatchedPayment;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"unmatched_payment\"");
    }

    #[test]
    fn payment_allocated_payload_has_allocation_lines() {
        let payload = PaymentAllocatedPayload {
            tenant_id: "t".to_string(),
            payment_id: "p".to_string(),
            customer_id: "c".to_string(),
            payment_amount_minor: 1000,
            allocated_amount_minor: 1000,
            unallocated_amount_minor: 0,
            currency: "usd".to_string(),
            allocation_strategy: "fifo".to_string(),
            allocations: vec![AllocationLine {
                invoice_id: "inv-1".to_string(),
                allocated_minor: 1000,
                remaining_after_minor: 0,
            }],
            allocated_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("allocation_strategy"));
        assert!(json.contains("allocations"));
        assert!(json.contains("remaining_after_minor"));
    }
}

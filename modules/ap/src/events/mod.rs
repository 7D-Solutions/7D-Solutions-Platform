//! AP event contracts v1
//!
//! Defines canonical event contracts for all AP domain events:
//! - ap.vendor_created / ap.vendor_updated
//! - ap.po_created / ap.po_approved / ap.po_closed / ap.po_line_received_linked
//! - ap.vendor_bill_created / ap.vendor_bill_matched / ap.vendor_bill_approved / ap.vendor_bill_voided
//! - ap.payment_run_created / ap.payment_executed
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0" (stable for v1)
//! - source_module: "ap"
//! - mutation_class: DATA_MUTATION, REVERSAL, or LIFECYCLE per event
//! - replay_safe: true (all payloads are self-contained at replay time)
//! - correlation_id / causation_id: caller-supplied for distributed tracing
//! - event_id: caller-supplied for idempotency (derive deterministically from business key)

pub mod bill;
pub mod envelope;
pub mod payment;
pub mod po;
pub mod vendor;
pub mod vendor_bill_approved;

// ============================================================================
// Shared Constants
// ============================================================================

/// Schema version for all AP event payloads (v1)
pub const AP_EVENT_SCHEMA_VERSION: &str = "1.0.0";

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

/// REVERSAL: compensates for a prior DATA_MUTATION (void, cancel)
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";

/// LIFECYCLE: entity lifecycle transitions (PO closed, status changes)
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

// ============================================================================
// Re-exports
// ============================================================================

pub use vendor::{
    VendorCreatedPayload, VendorUpdatedPayload,
    EVENT_TYPE_VENDOR_CREATED, EVENT_TYPE_VENDOR_UPDATED,
    build_vendor_created_envelope, build_vendor_updated_envelope,
};

pub use po::{
    PoLine, PoCreatedPayload, PoApprovedPayload, PoClosedPayload,
    PoLineReceivedLinkedPayload, PoCloseReason,
    EVENT_TYPE_PO_CREATED, EVENT_TYPE_PO_APPROVED,
    EVENT_TYPE_PO_CLOSED, EVENT_TYPE_PO_LINE_RECEIVED_LINKED,
    build_po_created_envelope, build_po_approved_envelope,
    build_po_closed_envelope, build_po_line_received_linked_envelope,
};

pub use bill::{
    BillLine, BillMatchLine, MatchType,
    VendorBillCreatedPayload, VendorBillMatchedPayload,
    VendorBillApprovedPayload, VendorBillVoidedPayload,
    EVENT_TYPE_VENDOR_BILL_CREATED, EVENT_TYPE_VENDOR_BILL_MATCHED,
    EVENT_TYPE_VENDOR_BILL_APPROVED, EVENT_TYPE_VENDOR_BILL_VOIDED,
    build_vendor_bill_created_envelope, build_vendor_bill_matched_envelope,
    build_vendor_bill_approved_envelope, build_vendor_bill_voided_envelope,
};

pub use vendor_bill_approved::ApprovedGlLine;

pub use payment::{
    PaymentRunItem, ApPaymentRunCreatedPayload, ApPaymentExecutedPayload,
    EVENT_TYPE_AP_PAYMENT_RUN_CREATED, EVENT_TYPE_AP_PAYMENT_EXECUTED,
    build_ap_payment_run_created_envelope, build_ap_payment_executed_envelope,
};

pub use envelope::EventEnvelope;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_is_stable() {
        assert_eq!(AP_EVENT_SCHEMA_VERSION, "1.0.0");
    }

    #[test]
    fn all_event_type_constants_use_ap_prefix() {
        assert!(EVENT_TYPE_VENDOR_CREATED.starts_with("ap."));
        assert!(EVENT_TYPE_VENDOR_UPDATED.starts_with("ap."));
        assert!(EVENT_TYPE_PO_CREATED.starts_with("ap."));
        assert!(EVENT_TYPE_PO_APPROVED.starts_with("ap."));
        assert!(EVENT_TYPE_PO_CLOSED.starts_with("ap."));
        assert!(EVENT_TYPE_PO_LINE_RECEIVED_LINKED.starts_with("ap."));
        assert!(EVENT_TYPE_VENDOR_BILL_CREATED.starts_with("ap."));
        assert!(EVENT_TYPE_VENDOR_BILL_MATCHED.starts_with("ap."));
        assert!(EVENT_TYPE_VENDOR_BILL_APPROVED.starts_with("ap."));
        assert!(EVENT_TYPE_VENDOR_BILL_VOIDED.starts_with("ap."));
        assert!(EVENT_TYPE_AP_PAYMENT_RUN_CREATED.starts_with("ap."));
        assert!(EVENT_TYPE_AP_PAYMENT_EXECUTED.starts_with("ap."));
    }
}

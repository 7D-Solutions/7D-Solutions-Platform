//! Vendor bill event contracts:
//!   ap.vendor_bill_created, ap.vendor_bill_matched,
//!   ap.vendor_bill_approved, ap.vendor_bill_voided

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_ap_envelope, EventEnvelope};
use crate::events::vendor_bill_approved::ApprovedGlLine;
use super::{AP_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_REVERSAL};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A vendor invoice/bill was entered into AP
pub const EVENT_TYPE_VENDOR_BILL_CREATED: &str = "ap.vendor_bill_created";

/// A vendor bill was matched to one or more PO lines (2-way or 3-way match)
pub const EVENT_TYPE_VENDOR_BILL_MATCHED: &str = "ap.vendor_bill_matched";

/// A vendor bill was approved for payment
pub const EVENT_TYPE_VENDOR_BILL_APPROVED: &str = "ap.vendor_bill_approved";

/// A vendor bill was voided (compensating event)
pub const EVENT_TYPE_VENDOR_BILL_VOIDED: &str = "ap.vendor_bill_voided";

// ============================================================================
// Shared types
// ============================================================================

/// A single line on a vendor bill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillLine {
    pub line_id: Uuid,
    pub description: String,
    pub quantity: f64,
    pub unit_price_minor: i64,
    pub line_total_minor: i64,
    pub gl_account_code: String,
    /// PO line this bill line references (if matched at entry)
    pub po_line_id: Option<Uuid>,
}

/// Match type for 2-way vs 3-way PO matching
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    /// PO ↔ Bill (no receipt)
    TwoWay,
    /// PO ↔ Receipt ↔ Bill
    ThreeWay,
    /// Bill only (no PO backing)
    NonPo,
}

// ============================================================================
// Payload: ap.vendor_bill_created
// ============================================================================

/// Payload for ap.vendor_bill_created
///
/// Self-contained: all line details, vendor reference, invoice number, and
/// due date included — no mutable state reads needed at replay time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorBillCreatedPayload {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    /// Vendor's invoice number (external reference)
    pub vendor_invoice_ref: String,
    /// ISO 4217 currency code
    pub currency: String,
    pub lines: Vec<BillLine>,
    /// Total bill amount in minor currency units
    pub total_minor: i64,
    /// Tax amount in minor currency units (if itemized)
    pub tax_minor: Option<i64>,
    pub invoice_date: DateTime<Utc>,
    pub due_date: DateTime<Utc>,
    pub entered_by: String,
    pub entered_at: DateTime<Utc>,
}

/// Build an envelope for ap.vendor_bill_created
pub fn build_vendor_bill_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorBillCreatedPayload,
) -> EventEnvelope<VendorBillCreatedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VENDOR_BILL_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.vendor_bill_matched
// ============================================================================

/// A single matched line linking bill ↔ PO line (↔ receipt)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillMatchLine {
    pub bill_line_id: Uuid,
    pub po_line_id: Uuid,
    /// Receipt record (required for 3-way match)
    pub receipt_id: Option<Uuid>,
    pub matched_quantity: f64,
    pub matched_amount_minor: i64,
    /// Whether quantity and price tolerance checks passed
    pub within_tolerance: bool,
}

/// Payload for ap.vendor_bill_matched
///
/// Emitted after 2-way or 3-way PO matching completes. Replay-safe: all
/// matched quantities and tolerance results are embedded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorBillMatchedPayload {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub po_id: Uuid,
    pub match_type: MatchType,
    pub match_lines: Vec<BillMatchLine>,
    /// True if all lines are within tolerance (bill ready for approval)
    pub fully_matched: bool,
    pub matched_by: String,
    pub matched_at: DateTime<Utc>,
}

/// Build an envelope for ap.vendor_bill_matched
pub fn build_vendor_bill_matched_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorBillMatchedPayload,
) -> EventEnvelope<VendorBillMatchedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VENDOR_BILL_MATCHED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.vendor_bill_approved
// ============================================================================

/// Payload for ap.vendor_bill_approved
///
/// Emitted when an approver signs off on a bill, queuing it for payment.
/// Carries all information needed by the GL consumer for posting without
/// re-reading the AP database (replay-safe).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorBillApprovedPayload {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub vendor_invoice_ref: String,
    /// Approved amount in minor currency units (may differ from bill total
    /// if partial approval; same total in the simple case)
    pub approved_amount_minor: i64,
    pub currency: String,
    pub due_date: DateTime<Utc>,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
    /// Phase 23a FX infrastructure identifier: UUID of the fx_rates row used
    /// when the bill was entered. None when bill currency == functional currency.
    pub fx_rate_id: Option<Uuid>,
    /// Per-line GL expense account allocations for replay-safe GL posting.
    /// Empty when the approval service could not populate line data.
    pub gl_lines: Vec<ApprovedGlLine>,
}

/// Build an envelope for ap.vendor_bill_approved
pub fn build_vendor_bill_approved_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorBillApprovedPayload,
) -> EventEnvelope<VendorBillApprovedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VENDOR_BILL_APPROVED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.vendor_bill_voided
// ============================================================================

/// Payload for ap.vendor_bill_voided
///
/// Compensating event for ap.vendor_bill_created. Carries the original bill
/// total so GL reversals can be posted without re-reading the original bill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorBillVoidedPayload {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub vendor_invoice_ref: String,
    /// Original bill total being reversed (minor currency units)
    pub original_total_minor: i64,
    pub currency: String,
    pub void_reason: String,
    pub voided_by: String,
    pub voided_at: DateTime<Utc>,
}

/// Build an envelope for ap.vendor_bill_voided
///
/// Sets reverses_event_id so downstream consumers can correlate the reversal.
pub fn build_vendor_bill_voided_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    reverses_event_id: Option<uuid::Uuid>,
    payload: VendorBillVoidedPayload,
) -> EventEnvelope<VendorBillVoidedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_VENDOR_BILL_VOIDED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
    .with_reverses_event_id(reverses_event_id)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bill_lines() -> Vec<BillLine> {
        vec![BillLine {
            line_id: Uuid::new_v4(),
            description: "Office supplies".to_string(),
            quantity: 5.0,
            unit_price_minor: 2000,
            line_total_minor: 10000,
            gl_account_code: "6200".to_string(),
            po_line_id: None,
        }]
    }

    #[test]
    fn vendor_bill_created_envelope_has_correct_metadata() {
        let payload = VendorBillCreatedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-042".to_string(),
            currency: "USD".to_string(),
            lines: sample_bill_lines(),
            total_minor: 10000,
            tax_minor: None,
            invoice_date: Utc::now(),
            due_date: Utc::now(),
            entered_by: "user-1".to_string(),
            entered_at: Utc::now(),
        };
        let envelope = build_vendor_bill_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_VENDOR_BILL_CREATED);
        assert_eq!(envelope.mutation_class.as_deref(), Some(MUTATION_CLASS_DATA_MUTATION));
        assert_eq!(envelope.source_module, "ap");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn vendor_bill_voided_uses_reversal_class_and_reverses_id() {
        let original_id = Uuid::new_v4();
        let payload = VendorBillVoidedPayload {
            bill_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: "INV-2026-042".to_string(),
            original_total_minor: 10000,
            currency: "USD".to_string(),
            void_reason: "Duplicate entry".to_string(),
            voided_by: "user-1".to_string(),
            voided_at: Utc::now(),
        };
        let envelope = build_vendor_bill_voided_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            Some(original_id),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_VENDOR_BILL_VOIDED);
        assert_eq!(envelope.mutation_class.as_deref(), Some(MUTATION_CLASS_REVERSAL));
        assert_eq!(envelope.reverses_event_id, Some(original_id));
    }

    #[test]
    fn match_type_serializes_to_snake_case() {
        let mt = MatchType::ThreeWay;
        let json = serde_json::to_string(&mt).unwrap();
        assert_eq!(json, "\"three_way\"");
    }
}

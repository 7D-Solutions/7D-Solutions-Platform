//! Purchase Order event contracts:
//!   ap.po_created, ap.po_approved, ap.po_closed, ap.po_line_received_linked

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AP_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_ap_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A purchase order was created and sent to a vendor
pub const EVENT_TYPE_PO_CREATED: &str = "ap.po_created";

/// A purchase order was approved for payment
pub const EVENT_TYPE_PO_APPROVED: &str = "ap.po_approved";

/// A purchase order was fully closed (all lines received or manually closed)
pub const EVENT_TYPE_PO_CLOSED: &str = "ap.po_closed";

/// A PO line was linked to a goods receipt (3-way match anchor)
pub const EVENT_TYPE_PO_LINE_RECEIVED_LINKED: &str = "ap.po_line_received_linked";

// ============================================================================
// Shared types
// ============================================================================

/// A single line on a purchase order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoLine {
    pub line_id: Uuid,
    /// Item or service description
    pub description: String,
    /// Quantity ordered
    pub quantity: f64,
    /// Unit of measure (e.g. "each", "kg")
    pub unit_of_measure: String,
    /// Unit price in minor currency units (e.g. cents)
    pub unit_price_minor: i64,
    /// GL expense account code
    pub gl_account_code: String,
}

// ============================================================================
// Payload: ap.po_created
// ============================================================================

/// Payload for ap.po_created
///
/// Self-contained: includes all line details needed to reconstruct the PO at replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoCreatedPayload {
    pub po_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    /// Human-readable PO number
    pub po_number: String,
    /// ISO 4217 currency code
    pub currency: String,
    pub lines: Vec<PoLine>,
    /// Total value in minor currency units
    pub total_minor: i64,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    /// Expected delivery date (if specified at creation)
    pub expected_delivery_date: Option<DateTime<Utc>>,
}

/// Build an envelope for ap.po_created
pub fn build_po_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PoCreatedPayload,
) -> EventEnvelope<PoCreatedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PO_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.po_approved
// ============================================================================

/// Payload for ap.po_approved
///
/// Emitted when a PO is approved, authorizing vendors to fulfill the order
/// and AP staff to match incoming bills against it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoApprovedPayload {
    pub po_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub po_number: String,
    /// Total approved amount in minor currency units
    pub approved_amount_minor: i64,
    pub currency: String,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
}

/// Build an envelope for ap.po_approved
pub fn build_po_approved_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PoApprovedPayload,
) -> EventEnvelope<PoApprovedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PO_APPROVED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.po_closed
// ============================================================================

/// Reason a PO was closed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PoCloseReason {
    /// All lines fully received and matched
    FullyReceived,
    /// Cancelled before any delivery
    Cancelled,
    /// Manually closed by AP staff (e.g. partial delivery accepted)
    ManualClose,
}

/// Payload for ap.po_closed
///
/// Lifecycle event — no financial mutation, just state transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoClosedPayload {
    pub po_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub po_number: String,
    pub close_reason: PoCloseReason,
    pub closed_by: String,
    pub closed_at: DateTime<Utc>,
}

/// Build an envelope for ap.po_closed
pub fn build_po_closed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PoClosedPayload,
) -> EventEnvelope<PoClosedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PO_CLOSED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ap.po_line_received_linked
// ============================================================================

/// Payload for ap.po_line_received_linked
///
/// Emitted when goods or services on a PO line are received and linked to a
/// receipt record. This is the 3-way match anchor connecting PO → receipt → bill.
/// Self-contained: includes qty, GL account, and receipt reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoLineReceivedLinkedPayload {
    pub po_id: Uuid,
    pub po_line_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    /// The receipt/GRN record this links to
    pub receipt_id: Uuid,
    pub quantity_received: f64,
    pub unit_of_measure: String,
    /// Unit price at time of PO creation (minor currency units)
    pub unit_price_minor: i64,
    pub currency: String,
    /// GL account for inventory or expense posting
    pub gl_account_code: String,
    pub received_at: DateTime<Utc>,
    pub received_by: String,
}

/// Build an envelope for ap.po_line_received_linked
pub fn build_po_line_received_linked_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PoLineReceivedLinkedPayload,
) -> EventEnvelope<PoLineReceivedLinkedPayload> {
    create_ap_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PO_LINE_RECEIVED_LINKED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AP_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_po_lines() -> Vec<PoLine> {
        vec![PoLine {
            line_id: Uuid::new_v4(),
            description: "Office chairs".to_string(),
            quantity: 10.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 45000,
            gl_account_code: "6100".to_string(),
        }]
    }

    #[test]
    fn po_created_envelope_has_correct_metadata() {
        let payload = PoCreatedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-001".to_string(),
            currency: "USD".to_string(),
            lines: sample_po_lines(),
            total_minor: 450000,
            created_by: "user-1".to_string(),
            created_at: Utc::now(),
            expected_delivery_date: None,
        };
        let envelope = build_po_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_PO_CREATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AP_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ap");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn po_closed_uses_lifecycle_mutation_class() {
        let payload = PoClosedPayload {
            po_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            vendor_id: Uuid::new_v4(),
            po_number: "PO-2026-001".to_string(),
            close_reason: PoCloseReason::FullyReceived,
            closed_by: "user-1".to_string(),
            closed_at: Utc::now(),
        };
        let envelope = build_po_closed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_PO_CLOSED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
    }

    #[test]
    fn po_line_received_linked_envelope_has_correct_type() {
        let payload = PoLineReceivedLinkedPayload {
            po_id: Uuid::new_v4(),
            po_line_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            vendor_id: Uuid::new_v4(),
            receipt_id: Uuid::new_v4(),
            quantity_received: 10.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 45000,
            currency: "USD".to_string(),
            gl_account_code: "6100".to_string(),
            received_at: Utc::now(),
            received_by: "user-1".to_string(),
        };
        let envelope = build_po_line_received_linked_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_PO_LINE_RECEIVED_LINKED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
    }

    #[test]
    fn po_close_reason_serializes_to_snake_case() {
        let reason = PoCloseReason::FullyReceived;
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, "\"fully_received\"");
    }
}

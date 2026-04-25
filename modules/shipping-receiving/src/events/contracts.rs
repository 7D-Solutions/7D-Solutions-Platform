//! Shipping-receiving event type constants and payload structs
//!
//! Defines the canonical event contracts for shipping-receiving events:
//! - shipping_receiving.shipment_created         (new shipment created)
//! - shipping_receiving.shipment_status_changed   (status transition occurred)
//! - shipping_receiving.inbound_closed            (inbound shipment fully received & closed)
//! - shipping_receiving.outbound_shipped          (outbound shipment shipped to carrier)
//! - shipping_receiving.outbound_delivered         (outbound shipment confirmed delivered)
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0"
//! - mutation_class: DATA_MUTATION
//! - correlation_id / causation_id: caller-supplied for tracing
//! - event_id: caller-supplied for idempotency (deterministic from business key)
//! - replay_safe: true

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::shipments::types::Direction;
use crate::events::create_shipping_receiving_envelope;
use event_bus::EventEnvelope;

use super::{MUTATION_CLASS_DATA_MUTATION, SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION};

// ============================================================================
// Event Type Constants
// ============================================================================

/// A new shipment (inbound or outbound) was created
pub const EVENT_TYPE_SHIPMENT_CREATED: &str = "shipping_receiving.shipment_created";

/// A shipment transitioned from one status to another
pub const EVENT_TYPE_SHIPMENT_STATUS_CHANGED: &str = "shipping_receiving.shipment_status_changed";

/// An inbound shipment was fully received and closed
pub const EVENT_TYPE_INBOUND_CLOSED: &str = "shipping_receiving.inbound_closed";

/// An outbound shipment was handed to the carrier
pub const EVENT_TYPE_OUTBOUND_SHIPPED: &str = "shipping_receiving.outbound_shipped";

/// An outbound shipment was confirmed delivered
pub const EVENT_TYPE_OUTBOUND_DELIVERED: &str = "shipping_receiving.outbound_delivered";

/// A receipt line was routed to inspection
pub const EVENT_TYPE_RECEIPT_ROUTED_TO_INSPECTION: &str = "sr.receipt_routed_to_inspection.v1";

/// A receipt line was routed direct to stock
pub const EVENT_TYPE_RECEIPT_ROUTED_TO_STOCK: &str = "sr.receipt_routed_to_stock.v1";

/// A carrier tracking event was received (webhook or poll) and persisted.
/// Downstream consumers use this for UI visibility updates and notifications.
pub const EVENT_TYPE_TRACKING_EVENT_RECEIVED: &str = "shipping_receiving.tracking.event_received";

// ============================================================================
// Payload: shipping_receiving.shipment_created
// ============================================================================

/// Payload for shipping_receiving.shipment_created
///
/// Emitted when a new shipment record is created in draft status.
/// Idempotency: caller MUST supply a deterministic event_id from the shipment key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipmentCreatedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub direction: Direction,
    /// Initial status as string (e.g. "draft")
    pub status: String,
    /// Party ID of the carrier, if known at creation time
    pub carrier_party_id: Option<Uuid>,
    pub tracking_number: Option<String>,
    /// Number of lines on the shipment at creation
    pub line_count: i64,
    pub created_at: DateTime<Utc>,
}

/// Build an envelope for shipping_receiving.shipment_created
pub fn build_shipment_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ShipmentCreatedPayload,
) -> EventEnvelope<ShipmentCreatedPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_SHIPMENT_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: shipping_receiving.shipment_status_changed
// ============================================================================

/// Payload for shipping_receiving.shipment_status_changed
///
/// Emitted on every state-machine transition. The old/new status are stored
/// as strings so consumers do not need to know about Direction-specific enums.
/// Idempotency: caller MUST supply a deterministic event_id from the transition key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipmentStatusChangedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub direction: Direction,
    pub old_status: String,
    pub new_status: String,
    /// User or system actor that triggered the transition
    pub changed_by: String,
    pub changed_at: DateTime<Utc>,
}

/// Build an envelope for shipping_receiving.shipment_status_changed
pub fn build_shipment_status_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ShipmentStatusChangedPayload,
) -> EventEnvelope<ShipmentStatusChangedPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_SHIPMENT_STATUS_CHANGED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: shipping_receiving.inbound_closed
// ============================================================================

/// One line from a closed inbound shipment, summarizing accepted/rejected quantities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundClosedLine {
    pub line_id: Uuid,
    pub sku: String,
    pub qty_accepted: i64,
    pub qty_rejected: i64,
    /// Receipt ID linking to the inventory receipt created for this line
    pub receipt_id: Option<Uuid>,
}

/// Payload for shipping_receiving.inbound_closed
///
/// Emitted when an inbound shipment transitions to Closed status after
/// all lines have been received and inspected. Inventory module consumes
/// this to create stock receipt records.
/// Idempotency: caller MUST supply a deterministic event_id from the shipment key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundClosedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub lines: Vec<InboundClosedLine>,
    pub closed_at: DateTime<Utc>,
}

/// Build an envelope for shipping_receiving.inbound_closed
pub fn build_inbound_closed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InboundClosedPayload,
) -> EventEnvelope<InboundClosedPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INBOUND_CLOSED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: shipping_receiving.outbound_shipped
// ============================================================================

/// One line from a shipped outbound shipment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundShippedLine {
    pub line_id: Uuid,
    pub sku: String,
    pub qty_shipped: i64,
    /// Issue ID linking to the inventory issue created for this line
    pub issue_id: Option<Uuid>,
    /// Source document type (e.g. "sales_order", "purchase_order")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref_type: Option<String>,
    /// Source document ID (e.g. the sales order UUID)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref_id: Option<Uuid>,
}

/// Payload for shipping_receiving.outbound_shipped
///
/// Emitted when an outbound shipment transitions to Shipped status after
/// being handed to the carrier. Inventory module consumes this to create
/// stock issue records.
/// Idempotency: caller MUST supply a deterministic event_id from the shipment key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundShippedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub lines: Vec<OutboundShippedLine>,
    pub shipped_at: DateTime<Utc>,
    /// Tracking number from the carrier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_number: Option<String>,
    /// Party ID of the carrier (UUID ref to Party module)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_party_id: Option<Uuid>,
}

/// Build an envelope for shipping_receiving.outbound_shipped
pub fn build_outbound_shipped_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OutboundShippedPayload,
) -> EventEnvelope<OutboundShippedPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_OUTBOUND_SHIPPED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: shipping_receiving.outbound_delivered
// ============================================================================

/// Payload for shipping_receiving.outbound_delivered
///
/// Emitted when an outbound shipment is confirmed delivered at destination.
/// Idempotency: caller MUST supply a deterministic event_id from the shipment key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundDeliveredPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub delivered_at: DateTime<Utc>,
}

/// Build an envelope for shipping_receiving.outbound_delivered
pub fn build_outbound_delivered_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OutboundDeliveredPayload,
) -> EventEnvelope<OutboundDeliveredPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_OUTBOUND_DELIVERED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: sr.receipt_routed_to_inspection.v1 / sr.receipt_routed_to_stock.v1
// ============================================================================

/// Payload for inspection routing events.
///
/// Emitted when a receiving line is routed to either stock or inspection.
/// The event_type distinguishes the routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptRoutedPayload {
    pub tenant_id: String,
    pub routing_id: Uuid,
    pub shipment_id: Uuid,
    pub shipment_line_id: Uuid,
    pub route_decision: String,
    pub reason: Option<String>,
    pub routed_by: Option<Uuid>,
    pub routed_at: DateTime<Utc>,
}

/// Build an envelope for a receipt routing event.
pub fn build_receipt_routed_envelope(
    event_id: Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReceiptRoutedPayload,
) -> EventEnvelope<ReceiptRoutedPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        event_type,
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

/// Shipping cost event contract (schema_version 1)
pub mod shipping_cost;

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod tests;

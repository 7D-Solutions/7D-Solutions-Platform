//! Shipping-receiving event contracts (schema_version = 1)
//!
//! Provides canonical event_type strings, payload structs, and envelope builder
//! helpers for all events emitted by the shipping-receiving module.
//!
//! ## Event Types
//!
//! | Event Type                                    | mutation_class | Consumer           |
//! |-----------------------------------------------|----------------|--------------------|
//! | shipping_receiving.shipment_created           | DATA_MUTATION  | projections        |
//! | shipping_receiving.shipment_status_changed    | DATA_MUTATION  | projections        |
//! | shipping_receiving.inbound_closed             | DATA_MUTATION  | inventory (receipt)|
//! | shipping_receiving.outbound_shipped           | DATA_MUTATION  | inventory (issue)  |
//! | shipping_receiving.outbound_delivered         | DATA_MUTATION  | projections        |
//! | shipping_receiving.shipping_cost.incurred     | DATA_MUTATION  | ap, ar             |

pub mod contracts;

// ============================================================================
// Shared Constants
// ============================================================================

/// Schema version for all shipping-receiving event payloads (v1)
pub const SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION: &str = "1.0.0";

/// DATA_MUTATION: creates or modifies a shipping-receiving record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

// ============================================================================
// Re-exports
// ============================================================================

pub use contracts::{
    build_inbound_closed_envelope, build_outbound_delivered_envelope,
    build_outbound_shipped_envelope, build_receipt_routed_envelope,
    build_shipment_created_envelope, build_shipment_status_changed_envelope, InboundClosedLine,
    InboundClosedPayload, OutboundDeliveredPayload, OutboundShippedLine, OutboundShippedPayload,
    ReceiptRoutedPayload, ShipmentCreatedPayload, ShipmentStatusChangedPayload,
    EVENT_TYPE_INBOUND_CLOSED, EVENT_TYPE_OUTBOUND_DELIVERED, EVENT_TYPE_OUTBOUND_SHIPPED,
    EVENT_TYPE_RECEIPT_ROUTED_TO_INSPECTION, EVENT_TYPE_RECEIPT_ROUTED_TO_STOCK,
    EVENT_TYPE_SHIPMENT_CREATED, EVENT_TYPE_SHIPMENT_STATUS_CHANGED,
    EVENT_TYPE_TRACKING_EVENT_RECEIVED, EVENT_TYPE_INBOUND_TRACKING_UPDATED,
};

pub use contracts::shipping_cost::{
    build_shipping_cost_incurred_envelope, ShippingCostIncurredPayload,
    EVENT_TYPE_SHIPPING_COST_INCURRED,
};

// ============================================================================
// Envelope builder helper
// ============================================================================

/// Create a shipping-receiving-scoped EventEnvelope.
///
/// Sets `source_module = "shipping-receiving"` and `replay_safe = true`.
/// Callers MUST supply a deterministic `event_id` derived from a stable business key.
pub fn create_shipping_receiving_envelope<T>(
    event_id: uuid::Uuid,
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    mutation_class: String,
    payload: T,
) -> event_bus::EventEnvelope<T> {
    event_bus::EventEnvelope::with_event_id(
        event_id,
        tenant_id,
        "shipping-receiving".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(correlation_id.clone()))
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(mutation_class))
    .with_replay_safe(true)
}

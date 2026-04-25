use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::{
    create_shipping_receiving_envelope, MUTATION_CLASS_DATA_MUTATION,
    SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION,
};
use event_bus::EventEnvelope;

// ============================================================================
// Event Type Constant
// ============================================================================

/// A carrier label was created and a shipping cost was incurred.
/// Emitted once per logical shipment (master, not per child package).
pub const EVENT_TYPE_SHIPPING_COST_INCURRED: &str = "shipping_receiving.shipping_cost.incurred";

// ============================================================================
// Payload: shipping_receiving.shipping_cost.incurred
// ============================================================================

/// Payload for shipping_receiving.shipping_cost.incurred (schema_version 1)
///
/// Emitted once per logical shipment when a carrier label is created.
/// - AP subscribes to create an open carrier obligation.
/// - AR subscribes to optionally add a shipping line to the customer invoice.
///
/// Invariant: one label = one event = one AP obligation + (optional) one AR line.
/// For multi-package shipments, emit from the master shipment only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShippingCostIncurredPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    /// Master tracking number; for multi-package, the master only (not per child).
    pub tracking_number: String,
    /// ups | fedex | usps | rl | xpo | odfl | saia
    pub carrier_code: String,
    /// Per-shipment billing reference for location attribution (carrier account).
    pub carrier_account_ref: Option<String>,
    /// "outbound" | "return"
    pub direction: String,
    /// What the carrier charged us (our AP cost), in minor currency units (e.g. cents).
    pub charge_minor: i64,
    /// What we bill the customer (AR line), None when shipping is free to customer.
    pub customer_charge_minor: Option<i64>,
    /// ISO 4217 currency code (e.g. "USD").
    pub currency: String,
    /// Links to sales-order or invoice in AR for automatic line attachment.
    pub order_ref: Option<String>,
    /// When the cost was incurred (label creation time).
    pub incurred_at: DateTime<Utc>,
    /// Caller-supplied correlation ID for end-to-end tracing.
    pub correlation_id: String,
}

// ============================================================================
// Envelope builder
// ============================================================================

/// Build an EventEnvelope for shipping_receiving.shipping_cost.incurred.
pub fn build_shipping_cost_incurred_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ShippingCostIncurredPayload,
) -> EventEnvelope<ShippingCostIncurredPayload> {
    create_shipping_receiving_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_SHIPPING_COST_INCURRED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION.to_string())
}

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
pub const EVENT_TYPE_SHIPMENT_STATUS_CHANGED: &str =
    "shipping_receiving.shipment_status_changed";

/// An inbound shipment was fully received and closed
pub const EVENT_TYPE_INBOUND_CLOSED: &str = "shipping_receiving.inbound_closed";

/// An outbound shipment was handed to the carrier
pub const EVENT_TYPE_OUTBOUND_SHIPPED: &str = "shipping_receiving.outbound_shipped";

/// An outbound shipment was confirmed delivered
pub const EVENT_TYPE_OUTBOUND_DELIVERED: &str = "shipping_receiving.outbound_delivered";

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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::shipments::types::{InboundStatus, OutboundStatus};
    use chrono::Utc;

    // ---- shipment_created ----

    #[test]
    fn shipment_created_envelope_has_correct_metadata() {
        let payload = ShipmentCreatedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            direction: Direction::Inbound,
            status: "draft".to_string(),
            carrier_party_id: Some(Uuid::new_v4()),
            tracking_number: Some("TRACK-001".to_string()),
            line_count: 3,
            created_at: Utc::now(),
        };
        let envelope = build_shipment_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_SHIPMENT_CREATED);
        assert_eq!(envelope.mutation_class.as_deref(), Some(MUTATION_CLASS_DATA_MUTATION));
        assert_eq!(envelope.schema_version, SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "shipping-receiving");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn shipment_created_payload_round_trip() {
        let payload = ShipmentCreatedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            direction: Direction::Outbound,
            status: "draft".to_string(),
            carrier_party_id: None,
            tracking_number: None,
            line_count: 0,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: ShipmentCreatedPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.tenant_id, "tenant-1");
        assert_eq!(decoded.direction, Direction::Outbound);
        assert_eq!(decoded.line_count, 0);
    }

    // ---- shipment_status_changed ----

    #[test]
    fn status_changed_envelope_has_correct_metadata() {
        let payload = ShipmentStatusChangedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            direction: Direction::Inbound,
            old_status: InboundStatus::Draft.as_str().to_string(),
            new_status: InboundStatus::Confirmed.as_str().to_string(),
            changed_by: "user-123".to_string(),
            changed_at: Utc::now(),
        };
        let envelope = build_shipment_status_changed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_SHIPMENT_STATUS_CHANGED);
        assert_eq!(envelope.mutation_class.as_deref(), Some(MUTATION_CLASS_DATA_MUTATION));
        assert_eq!(envelope.source_module, "shipping-receiving");
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
        assert!(envelope.replay_safe);
    }

    #[test]
    fn status_changed_payload_round_trip() {
        let payload = ShipmentStatusChangedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            direction: Direction::Outbound,
            old_status: OutboundStatus::Packed.as_str().to_string(),
            new_status: OutboundStatus::Shipped.as_str().to_string(),
            changed_by: "system".to_string(),
            changed_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: ShipmentStatusChangedPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.old_status, "packed");
        assert_eq!(decoded.new_status, "shipped");
    }

    // ---- inbound_closed ----

    #[test]
    fn inbound_closed_envelope_has_correct_metadata() {
        let payload = InboundClosedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            lines: vec![InboundClosedLine {
                line_id: Uuid::new_v4(),
                sku: "SKU-001".to_string(),
                qty_accepted: 100,
                qty_rejected: 2,
                receipt_id: Some(Uuid::new_v4()),
            }],
            closed_at: Utc::now(),
        };
        let envelope = build_inbound_closed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-3".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INBOUND_CLOSED);
        assert_eq!(envelope.source_module, "shipping-receiving");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn inbound_closed_payload_round_trip() {
        let lines = vec![
            InboundClosedLine {
                line_id: Uuid::new_v4(),
                sku: "SKU-A".to_string(),
                qty_accepted: 50,
                qty_rejected: 0,
                receipt_id: None,
            },
            InboundClosedLine {
                line_id: Uuid::new_v4(),
                sku: "SKU-B".to_string(),
                qty_accepted: 30,
                qty_rejected: 5,
                receipt_id: Some(Uuid::new_v4()),
            },
        ];
        let payload = InboundClosedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            lines,
            closed_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: InboundClosedPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.lines.len(), 2);
        assert_eq!(decoded.lines[0].sku, "SKU-A");
        assert_eq!(decoded.lines[1].qty_rejected, 5);
    }

    // ---- outbound_shipped ----

    #[test]
    fn outbound_shipped_envelope_has_correct_metadata() {
        let payload = OutboundShippedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            lines: vec![OutboundShippedLine {
                line_id: Uuid::new_v4(),
                sku: "SKU-001".to_string(),
                qty_shipped: 25,
                issue_id: Some(Uuid::new_v4()),
            }],
            shipped_at: Utc::now(),
        };
        let envelope = build_outbound_shipped_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-4".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_OUTBOUND_SHIPPED);
        assert_eq!(envelope.source_module, "shipping-receiving");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn outbound_shipped_payload_round_trip() {
        let payload = OutboundShippedPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            lines: vec![OutboundShippedLine {
                line_id: Uuid::new_v4(),
                sku: "SKU-X".to_string(),
                qty_shipped: 10,
                issue_id: None,
            }],
            shipped_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: OutboundShippedPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.lines.len(), 1);
        assert_eq!(decoded.lines[0].qty_shipped, 10);
    }

    // ---- outbound_delivered ----

    #[test]
    fn outbound_delivered_envelope_has_correct_metadata() {
        let payload = OutboundDeliveredPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            delivered_at: Utc::now(),
        };
        let envelope = build_outbound_delivered_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-5".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_OUTBOUND_DELIVERED);
        assert_eq!(envelope.source_module, "shipping-receiving");
        assert!(envelope.replay_safe);
    }

    #[test]
    fn outbound_delivered_payload_round_trip() {
        let delivered_at = Utc::now();
        let payload = OutboundDeliveredPayload {
            tenant_id: "tenant-1".to_string(),
            shipment_id: Uuid::new_v4(),
            delivered_at,
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let decoded: OutboundDeliveredPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.tenant_id, "tenant-1");
        assert_eq!(decoded.delivered_at, delivered_at);
    }

    // ---- all event type constants are distinct ----

    #[test]
    fn event_type_constants_are_unique() {
        let types = [
            EVENT_TYPE_SHIPMENT_CREATED,
            EVENT_TYPE_SHIPMENT_STATUS_CHANGED,
            EVENT_TYPE_INBOUND_CLOSED,
            EVENT_TYPE_OUTBOUND_SHIPPED,
            EVENT_TYPE_OUTBOUND_DELIVERED,
        ];
        for (i, a) in types.iter().enumerate() {
            for (j, b) in types.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate event type: {a}");
                }
            }
        }
    }
}

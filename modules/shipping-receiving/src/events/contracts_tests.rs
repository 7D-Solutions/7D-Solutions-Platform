use super::*;
use crate::domain::shipments::types::{InboundStatus, OutboundStatus};
use crate::events::{MUTATION_CLASS_DATA_MUTATION, SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION};
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
    assert_eq!(
        envelope.mutation_class.as_deref(),
        Some(MUTATION_CLASS_DATA_MUTATION)
    );
    assert_eq!(
        envelope.schema_version,
        SHIPPING_RECEIVING_EVENT_SCHEMA_VERSION
    );
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
    assert_eq!(
        envelope.mutation_class.as_deref(),
        Some(MUTATION_CLASS_DATA_MUTATION)
    );
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

//! All 9 events produced by the outside-processing module.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_op_envelope, EventEnvelope};
use super::{MUTATION_DATA, MUTATION_LIFECYCLE, MUTATION_REVERSAL};

// ── Event type constants (dot notation, no .v1 suffix) ──────────────────────

pub const EVENT_ORDER_CREATED: &str = "outside_processing.order_created";
pub const EVENT_ORDER_ISSUED: &str = "outside_processing.order_issued";
pub const EVENT_ORDER_CLOSED: &str = "outside_processing.order_closed";
pub const EVENT_ORDER_CANCELLED: &str = "outside_processing.order_cancelled";
pub const EVENT_SHIPMENT_REQUESTED: &str = "outside_processing.shipment_requested";
pub const EVENT_SHIPPED: &str = "outside_processing.shipped";
pub const EVENT_RETURNED: &str = "outside_processing.returned";
pub const EVENT_REVIEW_COMPLETED: &str = "outside_processing.review_completed";
pub const EVENT_RE_IDENTIFICATION_RECORDED: &str = "outside_processing.re_identification_recorded";

// ── order_created ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderCreatedPayload {
    pub op_order_id: Uuid,
    pub op_order_number: String,
    pub tenant_id: String,
    pub vendor_id: Option<Uuid>,
    pub service_type: Option<String>,
    pub work_order_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

pub fn build_order_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderCreatedPayload,
) -> EventEnvelope<OrderCreatedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_ORDER_CREATED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── order_issued ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderIssuedPayload {
    pub op_order_id: Uuid,
    pub tenant_id: String,
    pub purchase_order_id: Option<Uuid>,
    pub issued_at: DateTime<Utc>,
}

pub fn build_order_issued_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderIssuedPayload,
) -> EventEnvelope<OrderIssuedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_ORDER_ISSUED.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}

// ── order_closed ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderClosedPayload {
    pub op_order_id: Uuid,
    pub tenant_id: String,
    pub closed_at: DateTime<Utc>,
    pub final_accepted_qty: i32,
}

pub fn build_order_closed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderClosedPayload,
) -> EventEnvelope<OrderClosedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_ORDER_CLOSED.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}

// ── order_cancelled ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderCancelledPayload {
    pub op_order_id: Uuid,
    pub tenant_id: String,
    pub reason: Option<String>,
    pub cancelled_at: DateTime<Utc>,
}

pub fn build_order_cancelled_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderCancelledPayload,
) -> EventEnvelope<OrderCancelledPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_ORDER_CANCELLED.to_string(), correlation_id, causation_id, MUTATION_REVERSAL.to_string(), payload)
}

// ── shipment_requested ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipmentRequestedPayload {
    pub op_order_id: Uuid,
    pub ship_event_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Option<Uuid>,
    pub quantity_shipped: i32,
    pub lot_number: Option<String>,
    pub part_number: Option<String>,
}

pub fn build_shipment_requested_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ShipmentRequestedPayload,
) -> EventEnvelope<ShipmentRequestedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_SHIPMENT_REQUESTED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── shipped ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShippedPayload {
    pub op_order_id: Uuid,
    pub ship_event_id: Uuid,
    pub tenant_id: String,
    pub quantity_shipped: i32,
    pub ship_date: NaiveDate,
}

pub fn build_shipped_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ShippedPayload,
) -> EventEnvelope<ShippedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_SHIPPED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── returned ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReturnedPayload {
    pub op_order_id: Uuid,
    pub return_event_id: Uuid,
    pub tenant_id: String,
    pub quantity_received: i32,
    pub condition: String,
    pub received_date: NaiveDate,
}

pub fn build_returned_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReturnedPayload,
) -> EventEnvelope<ReturnedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_RETURNED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── review_completed ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCompletedPayload {
    pub op_order_id: Uuid,
    pub review_id: Uuid,
    pub tenant_id: String,
    pub outcome: String,
    pub reviewed_at: DateTime<Utc>,
}

pub fn build_review_completed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReviewCompletedPayload,
) -> EventEnvelope<ReviewCompletedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_REVIEW_COMPLETED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── re_identification_recorded ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReIdentificationRecordedPayload {
    pub op_order_id: Uuid,
    pub tenant_id: String,
    pub old_part_number: String,
    pub new_part_number: String,
    pub performed_at: DateTime<Utc>,
}

pub fn build_re_identification_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReIdentificationRecordedPayload,
) -> EventEnvelope<ReIdentificationRecordedPayload> {
    create_op_envelope(event_id, tenant_id, EVENT_RE_IDENTIFICATION_RECORDED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn event_type_constants_use_outside_processing_prefix() {
        for event_type in &[
            EVENT_ORDER_CREATED,
            EVENT_ORDER_ISSUED,
            EVENT_ORDER_CLOSED,
            EVENT_ORDER_CANCELLED,
            EVENT_SHIPMENT_REQUESTED,
            EVENT_SHIPPED,
            EVENT_RETURNED,
            EVENT_REVIEW_COMPLETED,
            EVENT_RE_IDENTIFICATION_RECORDED,
        ] {
            assert!(
                event_type.starts_with("outside_processing."),
                "Event '{}' must start with 'outside_processing.'",
                event_type
            );
            assert!(
                !event_type.contains(".v1"),
                "Event type '{}' must not contain .v1 suffix (only contract filenames use it)",
                event_type
            );
        }
    }

    #[test]
    fn order_created_envelope_has_correct_metadata() {
        let payload = OrderCreatedPayload {
            op_order_id: Uuid::new_v4(),
            op_order_number: "OP-000001".to_string(),
            tenant_id: "t1".to_string(),
            vendor_id: None,
            service_type: Some("heat_treat".to_string()),
            work_order_id: None,
            created_at: Utc::now(),
        };
        let env = build_order_created_envelope(
            Uuid::new_v4(), "t1".to_string(), "corr-1".to_string(), None, payload,
        );
        assert_eq!(env.event_type, EVENT_ORDER_CREATED);
        assert_eq!(env.source_module, "outside-processing");
        assert!(env.replay_safe);
    }
}

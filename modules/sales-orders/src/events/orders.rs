//! Sales order lifecycle event contracts.
//!
//! Event type strings use dot notation WITHOUT .v1 suffix in Rust code.
//! The .v1 suffix appears only in contract filenames and NATS subject registry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE, MUTATION_CLASS_REVERSAL,
    SO_EVENT_SCHEMA_VERSION,
};
use crate::events::envelope::{create_so_envelope, EventEnvelope};

// ── Event type constants ──────────────────────────────────────────────────────

pub const EVENT_TYPE_ORDER_CREATED: &str = "sales_orders.order_created";
pub const EVENT_TYPE_ORDER_BOOKED: &str = "sales_orders.order_booked";
pub const EVENT_TYPE_ORDER_CANCELLED: &str = "sales_orders.order_cancelled";
pub const EVENT_TYPE_ORDER_SHIPPED: &str = "sales_orders.order_shipped";
pub const EVENT_TYPE_ORDER_CLOSED: &str = "sales_orders.order_closed";

// ── Payloads ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderCreatedPayload {
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub customer_id: Option<Uuid>,
    pub currency: String,
    pub tenant_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookedLine {
    pub line_id: Uuid,
    pub item_id: Option<Uuid>,
    pub quantity: f64,
    pub required_date: Option<chrono::NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookedPayload {
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub customer_id: Option<Uuid>,
    pub total_cents: i64,
    pub currency: String,
    pub tenant_id: String,
    pub lines: Vec<BookedLine>,
    pub booked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderCancelledPayload {
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub tenant_id: String,
    pub reason: Option<String>,
    pub cancelled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderShippedPayload {
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub tenant_id: String,
    pub shipped_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderClosedPayload {
    pub sales_order_id: Uuid,
    pub order_number: String,
    pub tenant_id: String,
    pub closed_at: DateTime<Utc>,
}

// ── Envelope builders ─────────────────────────────────────────────────────────

pub fn build_order_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderCreatedPayload,
) -> EventEnvelope<OrderCreatedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ORDER_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_order_booked_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderBookedPayload,
) -> EventEnvelope<OrderBookedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ORDER_BOOKED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_order_cancelled_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderCancelledPayload,
) -> EventEnvelope<OrderCancelledPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ORDER_CANCELLED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_order_shipped_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderShippedPayload,
) -> EventEnvelope<OrderShippedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ORDER_SHIPPED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_order_closed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: OrderClosedPayload,
) -> EventEnvelope<OrderClosedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ORDER_CLOSED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

//! Cross-module event contracts: reservation.requested, shipment.requested, invoice.requested.
//! These are emitted by Sales-Orders and consumed by Inventory, Shipping-Receiving, and AR.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MUTATION_CLASS_DATA_MUTATION, SO_EVENT_SCHEMA_VERSION};
use crate::events::envelope::{create_so_envelope, EventEnvelope};

// ── Event type constants ──────────────────────────────────────────────────────

pub const EVENT_TYPE_RESERVATION_REQUESTED: &str = "sales_orders.reservation_requested";
pub const EVENT_TYPE_SHIPMENT_REQUESTED: &str = "sales_orders.shipment_requested";
pub const EVENT_TYPE_INVOICE_REQUESTED: &str = "sales_orders.invoice_requested";

// ── Payloads ─────────────────────────────────────────────────────────────────

/// Emitted per SO line on booking. Inventory consumes to hold stock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservationRequestedPayload {
    pub sales_order_id: Uuid,
    pub line_id: Uuid,
    pub item_id: Uuid,
    pub quantity: f64,
    pub required_date: Option<NaiveDate>,
    pub tenant_id: String,
}

/// Emitted when an SO line approaches promised_date or is explicitly triggered.
/// Shipping-Receiving consumes to create shipment documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipmentRequestedPayload {
    pub sales_order_id: Uuid,
    pub line_id: Uuid,
    pub item_id: Option<Uuid>,
    pub quantity: f64,
    pub ship_to_address_id: Option<Uuid>,
    pub tenant_id: String,
}

/// Emitted per SO line when shipped. AR consumes to create an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceRequestedPayload {
    pub sales_order_id: Uuid,
    pub line_id: Uuid,
    pub customer_id: Option<Uuid>,
    pub amount_cents: i64,
    pub currency: String,
    pub tenant_id: String,
    pub requested_at: DateTime<Utc>,
}

// ── Envelope builders ─────────────────────────────────────────────────────────

pub fn build_reservation_requested_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReservationRequestedPayload,
) -> EventEnvelope<ReservationRequestedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RESERVATION_REQUESTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_shipment_requested_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ShipmentRequestedPayload,
) -> EventEnvelope<ShipmentRequestedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_SHIPMENT_REQUESTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_invoice_requested_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceRequestedPayload,
) -> EventEnvelope<InvoiceRequestedPayload> {
    create_so_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_REQUESTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(SO_EVENT_SCHEMA_VERSION.to_string())
}

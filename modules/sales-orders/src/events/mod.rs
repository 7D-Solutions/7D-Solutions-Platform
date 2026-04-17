//! Sales-Orders event contracts v1
//!
//! All event_type strings use dot notation WITHOUT .v1 suffix in Rust code.
//! Follow the pattern from modules/ap/src/events/vendor.rs.
//!
//! Produced:
//!   sales_orders.order_created, .order_booked, .order_cancelled, .order_shipped, .order_closed
//!   sales_orders.blanket_activated, .blanket_expired, .blanket_cancelled
//!   sales_orders.release_created
//!   sales_orders.reservation_requested (cross-module → Inventory)
//!   sales_orders.shipment_requested    (cross-module → Shipping-Receiving)
//!   sales_orders.invoice_requested     (cross-module → AR)

pub mod blankets;
pub mod cross_module;
pub mod envelope;
pub mod orders;

// ── Shared constants ──────────────────────────────────────────────────────────

pub const SO_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use blankets::{
    build_blanket_activated_envelope, build_blanket_cancelled_envelope,
    build_blanket_expired_envelope, build_release_created_envelope, BlanketActivatedPayload,
    BlanketCancelledPayload, BlanketExpiredPayload, ReleaseCreatedPayload,
    EVENT_TYPE_BLANKET_ACTIVATED, EVENT_TYPE_BLANKET_CANCELLED, EVENT_TYPE_BLANKET_EXPIRED,
    EVENT_TYPE_RELEASE_CREATED,
};

pub use cross_module::{
    build_invoice_requested_envelope, build_reservation_requested_envelope,
    build_shipment_requested_envelope, InvoiceRequestedPayload, ReservationRequestedPayload,
    ShipmentRequestedPayload, EVENT_TYPE_INVOICE_REQUESTED, EVENT_TYPE_RESERVATION_REQUESTED,
    EVENT_TYPE_SHIPMENT_REQUESTED,
};

pub use orders::{
    build_order_booked_envelope, build_order_cancelled_envelope, build_order_closed_envelope,
    build_order_created_envelope, build_order_shipped_envelope, BookedLine, OrderBookedPayload,
    OrderCancelledPayload, OrderClosedPayload, OrderCreatedPayload, OrderShippedPayload,
    EVENT_TYPE_ORDER_BOOKED, EVENT_TYPE_ORDER_CANCELLED, EVENT_TYPE_ORDER_CLOSED,
    EVENT_TYPE_ORDER_CREATED, EVENT_TYPE_ORDER_SHIPPED,
};

pub use envelope::EventEnvelope;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_is_stable() {
        assert_eq!(SO_EVENT_SCHEMA_VERSION, "1.0.0");
    }

    #[test]
    fn all_event_type_constants_use_sales_orders_prefix() {
        let events = [
            EVENT_TYPE_ORDER_CREATED,
            EVENT_TYPE_ORDER_BOOKED,
            EVENT_TYPE_ORDER_CANCELLED,
            EVENT_TYPE_ORDER_SHIPPED,
            EVENT_TYPE_ORDER_CLOSED,
            EVENT_TYPE_BLANKET_ACTIVATED,
            EVENT_TYPE_BLANKET_EXPIRED,
            EVENT_TYPE_BLANKET_CANCELLED,
            EVENT_TYPE_RELEASE_CREATED,
            EVENT_TYPE_RESERVATION_REQUESTED,
            EVENT_TYPE_SHIPMENT_REQUESTED,
            EVENT_TYPE_INVOICE_REQUESTED,
        ];
        for evt in &events {
            assert!(
                evt.starts_with("sales_orders."),
                "Event type '{}' must start with 'sales_orders.'",
                evt
            );
        }
    }

    #[test]
    fn event_types_have_no_v1_suffix() {
        let events = [
            EVENT_TYPE_ORDER_CREATED,
            EVENT_TYPE_ORDER_BOOKED,
            EVENT_TYPE_BLANKET_ACTIVATED,
            EVENT_TYPE_RESERVATION_REQUESTED,
        ];
        for evt in &events {
            assert!(
                !evt.ends_with(".v1"),
                "Event type '{}' must not include .v1 suffix in Rust code",
                evt
            );
        }
    }
}

//! Contract validation tests for sales-orders events.
//!
//! Each test builds an envelope via the real emitter, serializes it to JSON,
//! and validates it against the corresponding JSON Schema contract file in
//! contracts/events/. No mocks, no golden files — the live emitter IS the test.

use chrono::{NaiveDate, Utc};
use jsonschema::JSONSchema;
use sales_orders_rs::events::{
    blankets::{
        build_blanket_activated_envelope, build_blanket_cancelled_envelope,
        build_blanket_expired_envelope, build_release_created_envelope, BlanketActivatedPayload,
        BlanketCancelledPayload, BlanketExpiredPayload, ReleaseCreatedPayload,
    },
    cross_module::{
        build_invoice_requested_envelope, build_reservation_requested_envelope,
        build_shipment_requested_envelope, InvoiceRequestedPayload, ReservationRequestedPayload,
        ShipmentRequestedPayload,
    },
    orders::{
        build_order_booked_envelope, build_order_cancelled_envelope, build_order_closed_envelope,
        build_order_created_envelope, build_order_shipped_envelope, BookedLine, OrderBookedPayload,
        OrderCancelledPayload, OrderClosedPayload, OrderCreatedPayload, OrderShippedPayload,
    },
};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("contracts/events")
}

fn load_schema(event_slug: &str) -> JSONSchema {
    let path = contracts_dir().join(format!("sales-orders-{}.v1.json", event_slug));
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Cannot read schema at {:?}", path));
    let schema_val: Value = serde_json::from_str(&contents)
        .unwrap_or_else(|_| panic!("Invalid JSON in schema {:?}", path));
    JSONSchema::compile(&schema_val)
        .unwrap_or_else(|e| panic!("Schema compile error for {}: {}", event_slug, e))
}

fn assert_valid<T: Serialize>(schema: &JSONSchema, envelope: &T, label: &str) {
    let value = serde_json::to_value(envelope)
        .unwrap_or_else(|e| panic!("Serialization failed for {}: {}", label, e));
    let msgs: Vec<String> = match schema.validate(&value) {
        Ok(()) => vec![],
        Err(errors) => errors
            .map(|e| format!("  [{}] {}", e.instance_path, e))
            .collect(),
    };
    if !msgs.is_empty() {
        panic!("{} failed schema validation:\n{}", label, msgs.join("\n"));
    }
}

fn tenant() -> String {
    "test-tenant-contract-001".to_string()
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

#[test]
fn order_created_validates_against_contract() {
    let schema = load_schema("order-created");
    let payload = OrderCreatedPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-001".to_string(),
        customer_id: Some(Uuid::new_v4()),
        currency: "USD".to_string(),
        tenant_id: tenant(),
        created_at: Utc::now(),
    };
    let env = build_order_created_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_created");
}

#[test]
fn order_created_null_customer_validates_against_contract() {
    let schema = load_schema("order-created");
    let payload = OrderCreatedPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-002".to_string(),
        customer_id: None,
        currency: "EUR".to_string(),
        tenant_id: tenant(),
        created_at: Utc::now(),
    };
    let env = build_order_created_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_created (null customer_id)");
}

#[test]
fn order_booked_validates_against_contract() {
    let schema = load_schema("order-booked");
    let payload = OrderBookedPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-003".to_string(),
        customer_id: Some(Uuid::new_v4()),
        total_cents: 150_00,
        currency: "USD".to_string(),
        tenant_id: tenant(),
        lines: vec![
            BookedLine {
                line_id: Uuid::new_v4(),
                item_id: Some(Uuid::new_v4()),
                quantity: 5.0,
                required_date: Some(NaiveDate::from_ymd_opt(2024, 6, 30).unwrap()),
            },
            BookedLine {
                line_id: Uuid::new_v4(),
                item_id: None,
                quantity: 1.0,
                required_date: None,
            },
        ],
        booked_at: Utc::now(),
    };
    let env = build_order_booked_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_booked");
}

#[test]
fn order_cancelled_validates_against_contract() {
    let schema = load_schema("order-cancelled");
    let payload = OrderCancelledPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-004".to_string(),
        tenant_id: tenant(),
        reason: Some("Customer request".to_string()),
        cancelled_at: Utc::now(),
    };
    let env = build_order_cancelled_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_cancelled");
}

#[test]
fn order_cancelled_null_reason_validates_against_contract() {
    let schema = load_schema("order-cancelled");
    let payload = OrderCancelledPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-005".to_string(),
        tenant_id: tenant(),
        reason: None,
        cancelled_at: Utc::now(),
    };
    let env = build_order_cancelled_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_cancelled (null reason)");
}

#[test]
fn order_shipped_validates_against_contract() {
    let schema = load_schema("order-shipped");
    let payload = OrderShippedPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-006".to_string(),
        tenant_id: tenant(),
        shipped_at: Utc::now(),
    };
    let env = build_order_shipped_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_shipped");
}

#[test]
fn order_closed_validates_against_contract() {
    let schema = load_schema("order-closed");
    let payload = OrderClosedPayload {
        sales_order_id: Uuid::new_v4(),
        order_number: "SO-2024-007".to_string(),
        tenant_id: tenant(),
        closed_at: Utc::now(),
    };
    let env = build_order_closed_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "order_closed");
}

#[test]
fn blanket_activated_validates_against_contract() {
    let schema = load_schema("blanket-activated");
    let payload = BlanketActivatedPayload {
        blanket_order_id: Uuid::new_v4(),
        blanket_order_number: "BO-2024-001".to_string(),
        customer_id: Some(Uuid::new_v4()),
        total_committed_value_cents: 500_000_00,
        valid_until: Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
        tenant_id: tenant(),
        activated_at: Utc::now(),
    };
    let env = build_blanket_activated_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "blanket_activated");
}

#[test]
fn blanket_activated_null_optionals_validates_against_contract() {
    let schema = load_schema("blanket-activated");
    let payload = BlanketActivatedPayload {
        blanket_order_id: Uuid::new_v4(),
        blanket_order_number: "BO-2024-002".to_string(),
        customer_id: None,
        total_committed_value_cents: 100_00,
        valid_until: None,
        tenant_id: tenant(),
        activated_at: Utc::now(),
    };
    let env = build_blanket_activated_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "blanket_activated (null optionals)");
}

#[test]
fn blanket_expired_validates_against_contract() {
    let schema = load_schema("blanket-expired");
    let payload = BlanketExpiredPayload {
        blanket_order_id: Uuid::new_v4(),
        blanket_order_number: "BO-2024-003".to_string(),
        tenant_id: tenant(),
        expired_at: Utc::now(),
    };
    let env = build_blanket_expired_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "blanket_expired");
}

#[test]
fn blanket_cancelled_validates_against_contract() {
    let schema = load_schema("blanket-cancelled");
    let payload = BlanketCancelledPayload {
        blanket_order_id: Uuid::new_v4(),
        blanket_order_number: "BO-2024-004".to_string(),
        tenant_id: tenant(),
        cancelled_at: Utc::now(),
    };
    let env = build_blanket_cancelled_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "blanket_cancelled");
}

#[test]
fn release_created_validates_against_contract() {
    let schema = load_schema("release-created");
    let payload = ReleaseCreatedPayload {
        release_id: Uuid::new_v4(),
        blanket_order_id: Uuid::new_v4(),
        blanket_order_line_id: Uuid::new_v4(),
        release_qty: 10.0,
        sales_order_id: Uuid::new_v4(),
        tenant_id: tenant(),
        created_at: Utc::now(),
    };
    let env = build_release_created_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "release_created");
}

#[test]
fn reservation_requested_validates_against_contract() {
    let schema = load_schema("reservation-requested");
    let payload = ReservationRequestedPayload {
        sales_order_id: Uuid::new_v4(),
        line_id: Uuid::new_v4(),
        item_id: Uuid::new_v4(),
        quantity: 3.5,
        required_date: Some(NaiveDate::from_ymd_opt(2024, 7, 15).unwrap()),
        tenant_id: tenant(),
    };
    let env = build_reservation_requested_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "reservation_requested");
}

#[test]
fn reservation_requested_null_date_validates_against_contract() {
    let schema = load_schema("reservation-requested");
    let payload = ReservationRequestedPayload {
        sales_order_id: Uuid::new_v4(),
        line_id: Uuid::new_v4(),
        item_id: Uuid::new_v4(),
        quantity: 1.0,
        required_date: None,
        tenant_id: tenant(),
    };
    let env = build_reservation_requested_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "reservation_requested (null date)");
}

#[test]
fn shipment_requested_validates_against_contract() {
    let schema = load_schema("shipment-requested");
    let payload = ShipmentRequestedPayload {
        sales_order_id: Uuid::new_v4(),
        line_id: Uuid::new_v4(),
        item_id: Some(Uuid::new_v4()),
        quantity: 2.0,
        ship_to_address_id: Some(Uuid::new_v4()),
        tenant_id: tenant(),
    };
    let env = build_shipment_requested_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "shipment_requested");
}

#[test]
fn shipment_requested_null_optionals_validates_against_contract() {
    let schema = load_schema("shipment-requested");
    let payload = ShipmentRequestedPayload {
        sales_order_id: Uuid::new_v4(),
        line_id: Uuid::new_v4(),
        item_id: None,
        quantity: 1.0,
        ship_to_address_id: None,
        tenant_id: tenant(),
    };
    let env = build_shipment_requested_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "shipment_requested (null optionals)");
}

#[test]
fn invoice_requested_validates_against_contract() {
    let schema = load_schema("invoice-requested");
    let payload = InvoiceRequestedPayload {
        sales_order_id: Uuid::new_v4(),
        line_id: Uuid::new_v4(),
        customer_id: Some(Uuid::new_v4()),
        amount_cents: 1_299_00,
        currency: "USD".to_string(),
        tenant_id: tenant(),
        requested_at: Utc::now(),
    };
    let env = build_invoice_requested_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "invoice_requested");
}

#[test]
fn invoice_requested_null_customer_validates_against_contract() {
    let schema = load_schema("invoice-requested");
    let payload = InvoiceRequestedPayload {
        sales_order_id: Uuid::new_v4(),
        line_id: Uuid::new_v4(),
        customer_id: None,
        amount_cents: 500_00,
        currency: "EUR".to_string(),
        tenant_id: tenant(),
        requested_at: Utc::now(),
    };
    let env = build_invoice_requested_envelope(Uuid::new_v4(), tenant(), corr(), None, payload);
    assert_valid(&schema, &env, "invoice_requested (null customer_id)");
}

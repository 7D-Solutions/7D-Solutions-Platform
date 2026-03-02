//! Integration tests for outbox relay: validate → serialize → publish → consume.

use event_bus::outbox::validate_and_serialize_envelope;
use event_bus::{EventBus, EventEnvelope, InMemoryBus};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct OrderCreated {
    order_id: String,
    amount: i64,
}

/// Helper: build a valid envelope with mutation_class set.
fn make_envelope(
    tenant: &str,
    event_type: &str,
    payload: OrderCreated,
) -> EventEnvelope<OrderCreated> {
    EventEnvelope::new(
        tenant.to_string(),
        "orders".to_string(),
        event_type.to_string(),
        payload,
    )
    .with_mutation_class(Some("DATA_MUTATION".to_string()))
}

#[tokio::test]
async fn outbox_validates_serializes_and_publishes() {
    let envelope = make_envelope(
        "tenant-1",
        "order.created",
        OrderCreated {
            order_id: "o-100".into(),
            amount: 5000,
        },
    );

    // Step 1: outbox validation + serialization
    let validated = validate_and_serialize_envelope(&envelope).expect("validation should pass");

    assert_eq!(validated["tenant_id"], "tenant-1");
    assert_eq!(validated["event_type"], "order.created");
    assert_eq!(validated["payload"]["order_id"], "o-100");

    // Step 2: publish the serialized payload over the bus
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("orders.events.>").await.unwrap();

    let bytes = serde_json::to_vec(&validated).unwrap();
    bus.publish("orders.events.order.created", bytes.clone())
        .await
        .unwrap();

    // Step 3: consumer picks up the message
    let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("should not timeout")
        .expect("stream should yield a message");

    assert_eq!(msg.subject, "orders.events.order.created");

    // Deserialize and verify round-trip
    let received: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
    assert_eq!(received["tenant_id"], "tenant-1");
    assert_eq!(received["payload"]["amount"], 5000);
}

#[tokio::test]
async fn outbox_rejects_invalid_envelope_before_publish() {
    // Missing mutation_class → validation must fail, nothing published
    let envelope = EventEnvelope::new(
        "tenant-1".to_string(),
        "orders".to_string(),
        "order.created".to_string(),
        OrderCreated {
            order_id: "o-101".into(),
            amount: 100,
        },
    );
    // mutation_class is None

    let result = validate_and_serialize_envelope(&envelope);
    assert!(result.is_err(), "should reject missing mutation_class");
    assert!(result.unwrap_err().contains("mutation_class"));
}

#[tokio::test]
async fn outbox_relay_publishes_multiple_events_in_order() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("orders.events.>").await.unwrap();

    // Simulate an outbox relay draining 5 events
    for i in 0..5 {
        let envelope = make_envelope(
            "tenant-1",
            &format!("order.item_{}", i),
            OrderCreated {
                order_id: format!("o-{}", i),
                amount: (i + 1) * 1000,
            },
        );
        let validated = validate_and_serialize_envelope(&envelope).unwrap();
        let bytes = serde_json::to_vec(&validated).unwrap();
        bus.publish(&format!("orders.events.order.item_{}", i), bytes)
            .await
            .unwrap();
    }

    // Verify all 5 arrive in order
    for i in 0..5 {
        let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timeout")
            .expect("stream ended");

        assert_eq!(msg.subject, format!("orders.events.order.item_{}", i));
        let body: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
        assert_eq!(body["payload"]["order_id"], format!("o-{}", i));
    }
}

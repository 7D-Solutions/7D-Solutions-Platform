//! Integration tests for idempotency: duplicate delivery is a no-op.

use event_bus::outbox::validate_and_serialize_envelope;
use event_bus::{EventBus, EventEnvelope, InMemoryBus};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaymentProcessed {
    payment_id: String,
    amount: i64,
}

/// Simulates a consumer that tracks seen event_ids and skips duplicates.
struct IdempotentConsumer {
    seen: HashSet<String>,
    processed_count: usize,
}

impl IdempotentConsumer {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            processed_count: 0,
        }
    }

    /// Returns true if the event was processed (first time), false if duplicate.
    fn handle(&mut self, payload: &[u8]) -> bool {
        let envelope: serde_json::Value = serde_json::from_slice(payload).unwrap();
        let event_id = envelope["event_id"].as_str().unwrap().to_string();

        if self.seen.contains(&event_id) {
            // Duplicate — skip
            return false;
        }
        self.seen.insert(event_id);
        self.processed_count += 1;
        true
    }
}

#[tokio::test]
async fn duplicate_event_id_is_skipped_by_consumer() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("payments.>").await.unwrap();

    let event_id = Uuid::new_v4();
    let envelope = EventEnvelope::with_event_id(
        event_id,
        "tenant-1".to_string(),
        "payments".to_string(),
        "payment.processed".to_string(),
        PaymentProcessed {
            payment_id: "pay-1".into(),
            amount: 2500,
        },
    )
    .with_mutation_class(Some("DATA_MUTATION".to_string()));

    let validated = validate_and_serialize_envelope(&envelope).unwrap();
    let bytes = serde_json::to_vec(&validated).unwrap();

    // Publish the same event 3 times (simulating at-least-once delivery)
    for _ in 0..3 {
        bus.publish("payments.processed", bytes.clone())
            .await
            .unwrap();
    }

    let mut consumer = IdempotentConsumer::new();

    // Drain all 3 messages
    for _ in 0..3 {
        let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();
        consumer.handle(&msg.payload);
    }

    // Only 1 processed, 2 were duplicates
    assert_eq!(consumer.processed_count, 1);
    assert_eq!(consumer.seen.len(), 1);
    assert!(consumer.seen.contains(&event_id.to_string()));
}

#[tokio::test]
async fn distinct_event_ids_are_all_processed() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("payments.>").await.unwrap();

    // Publish 5 events each with a unique event_id
    for i in 0..5 {
        let envelope = EventEnvelope::new(
            "tenant-1".to_string(),
            "payments".to_string(),
            "payment.processed".to_string(),
            PaymentProcessed {
                payment_id: format!("pay-{}", i),
                amount: (i + 1) * 1000,
            },
        )
        .with_mutation_class(Some("DATA_MUTATION".to_string()));

        let validated = validate_and_serialize_envelope(&envelope).unwrap();
        let bytes = serde_json::to_vec(&validated).unwrap();
        bus.publish("payments.processed", bytes).await.unwrap();
    }

    let mut consumer = IdempotentConsumer::new();
    for _ in 0..5 {
        let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();
        consumer.handle(&msg.payload);
    }

    assert_eq!(consumer.processed_count, 5);
    assert_eq!(consumer.seen.len(), 5);
}

#[tokio::test]
async fn mixed_duplicates_and_unique_events() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("payments.>").await.unwrap();

    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();

    // Publish: A, B, A, B, A  (3 copies of A, 2 copies of B)
    let ids = [id_a, id_b, id_a, id_b, id_a];
    for (i, &eid) in ids.iter().enumerate() {
        let envelope = EventEnvelope::with_event_id(
            eid,
            "tenant-1".to_string(),
            "payments".to_string(),
            "payment.processed".to_string(),
            PaymentProcessed {
                payment_id: format!("pay-{}", i),
                amount: 100,
            },
        )
        .with_mutation_class(Some("DATA_MUTATION".to_string()));

        let validated = validate_and_serialize_envelope(&envelope).unwrap();
        let bytes = serde_json::to_vec(&validated).unwrap();
        bus.publish("payments.processed", bytes).await.unwrap();
    }

    let mut consumer = IdempotentConsumer::new();
    for _ in 0..5 {
        let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();
        consumer.handle(&msg.payload);
    }

    // 2 unique events processed
    assert_eq!(consumer.processed_count, 2);
    assert!(consumer.seen.contains(&id_a.to_string()));
    assert!(consumer.seen.contains(&id_b.to_string()));
}

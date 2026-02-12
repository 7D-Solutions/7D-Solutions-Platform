mod common;

use ar_rs::events::{
    enqueue_event, is_event_processed, mark_event_processed, process_event_idempotent,
    EventEnvelope,
};
use event_bus::{EventBus, InMemoryBus};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestEventData {
    message: String,
    amount: i64,
}

/// TEST 1: Enqueue event to outbox
#[tokio::test]
#[serial]
async fn test_enqueue_event_to_outbox() {
    let pool = common::setup_pool().await;

    // Clean up outbox
    sqlx::query("DELETE FROM events_outbox")
        .execute(&pool)
        .await
        .ok();

    // Create event envelope
    let event_data = TestEventData {
        message: "Test event".to_string(),
        amount: 100,
    };

    let envelope = EventEnvelope::new(
        "ar.invoice.created".to_string(),
        "1.0.0".to_string(),
        "tenant-123".to_string(),
        "invoice".to_string(),
        "inv-456".to_string(),
        "trace-789".to_string(),
        event_data,
    );

    let event_id = envelope.event_id;

    // Enqueue event
    let result = enqueue_event(&pool, &envelope).await;
    assert!(result.is_ok(), "Should enqueue event successfully");

    // Verify event is in outbox
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) as count
        FROM events_outbox
        WHERE event_id = $1 AND published_at IS NULL
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("Should query outbox");

    assert_eq!(row.0, 1, "Event should be in outbox and unpublished");

    // Clean up
    sqlx::query("DELETE FROM events_outbox WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();

    common::teardown_pool(pool).await;
}

/// TEST 2: Check idempotent consumer - first processing
#[tokio::test]
#[serial]
async fn test_idempotent_consumer_first_processing() {
    let pool = common::setup_pool().await;

    let event_id = Uuid::new_v4();

    // Clean up processed_events
    sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();

    // Check if event is processed (should be false)
    let is_processed = is_event_processed(&pool, event_id)
        .await
        .expect("Should check if event is processed");

    assert!(!is_processed, "Event should not be processed yet");

    // Mark as processed
    mark_event_processed(&pool, event_id, "ar.test.event", "ar-rs")
        .await
        .expect("Should mark event as processed");

    // Check again (should be true now)
    let is_processed = is_event_processed(&pool, event_id)
        .await
        .expect("Should check if event is processed");

    assert!(is_processed, "Event should be marked as processed");

    // Clean up
    sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();

    common::teardown_pool(pool).await;
}

/// TEST 3: Idempotent consumer - duplicate detection
#[tokio::test]
#[serial]
async fn test_idempotent_consumer_duplicate_detection() {
    let pool = common::setup_pool().await;

    let event_id = Uuid::new_v4();

    // Clean up processed_events
    sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();

    let mut call_count = 0;

    // First call - should process
    let processed = process_event_idempotent(
        &pool,
        event_id,
        "ar.test.duplicate",
        "ar-rs",
        || async {
            call_count += 1;
            Ok::<(), Box<dyn std::error::Error>>(())
        },
    )
    .await
    .expect("Should process event");

    assert!(processed, "First call should process the event");
    assert_eq!(call_count, 1, "Handler should be called once");

    // Second call - should detect duplicate and skip
    let processed = process_event_idempotent(
        &pool,
        event_id,
        "ar.test.duplicate",
        "ar-rs",
        || async {
            call_count += 1;
            Ok::<(), Box<dyn std::error::Error>>(())
        },
    )
    .await
    .expect("Should detect duplicate");

    assert!(!processed, "Second call should detect duplicate");
    assert_eq!(call_count, 1, "Handler should not be called again");

    // Clean up
    sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();

    common::teardown_pool(pool).await;
}

/// TEST 4: Event envelope with causation tracking
#[tokio::test]
async fn test_event_envelope_with_causation() {
    let event_data = TestEventData {
        message: "Caused event".to_string(),
        amount: 200,
    };

    let causation_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        "ar.payment.processed".to_string(),
        "1.0.0".to_string(),
        "tenant-456".to_string(),
        "payment".to_string(),
        "pay-789".to_string(),
        "trace-abc".to_string(),
        event_data,
    )
    .with_causation(causation_id);

    assert_eq!(envelope.causation_id, Some(causation_id));
    assert_eq!(envelope.producer, "ar-rs");
    assert_eq!(envelope.event_type, "ar.payment.processed");
    assert_eq!(envelope.tenant_id, "tenant-456");
}

/// TEST 5: Publisher integration test with InMemoryBus
#[tokio::test]
#[serial]
async fn test_publisher_with_inmemory_bus() {
    let pool = common::setup_pool().await;
    let event_bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());

    // Clean up outbox
    sqlx::query("DELETE FROM events_outbox")
        .execute(&pool)
        .await
        .ok();

    // Create and enqueue test event
    let event_data = TestEventData {
        message: "Publisher test".to_string(),
        amount: 300,
    };

    let envelope = EventEnvelope::new(
        "ar.test.published".to_string(),
        "1.0.0".to_string(),
        "tenant-789".to_string(),
        "test".to_string(),
        "test-123".to_string(),
        "trace-xyz".to_string(),
        event_data,
    );

    let event_id = envelope.event_id;

    enqueue_event(&pool, &envelope)
        .await
        .expect("Should enqueue event");

    // Import publisher functions
    use ar_rs::events::outbox::{fetch_unpublished_events, mark_as_published};

    // Manually run one publisher cycle
    let events = fetch_unpublished_events(&pool, 100)
        .await
        .expect("Should fetch unpublished events");

    assert!(!events.is_empty(), "Should have unpublished event");

    for event in events {
        let subject = format!("ar.events.{}", event.event_type.replace('.', "."));
        let payload = serde_json::to_vec(&event.payload).expect("Should serialize");

        event_bus
            .publish(&subject, payload)
            .await
            .expect("Should publish to bus");

        mark_as_published(&pool, event.event_id)
            .await
            .expect("Should mark as published");
    }

    // Verify event is marked as published
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) as count
        FROM events_outbox
        WHERE event_id = $1 AND published_at IS NOT NULL
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("Should query outbox");

    assert_eq!(row.0, 1, "Event should be marked as published");

    // Clean up
    sqlx::query("DELETE FROM events_outbox WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();

    common::teardown_pool(pool).await;
}

/// TEST 6: Multiple events batch processing
#[tokio::test]
#[serial]
async fn test_batch_event_enqueue() {
    let pool = common::setup_pool().await;

    // Clean up outbox
    sqlx::query("DELETE FROM events_outbox")
        .execute(&pool)
        .await
        .ok();

    let mut event_ids = Vec::new();

    // Enqueue multiple events
    for i in 0..5 {
        let event_data = TestEventData {
            message: format!("Batch event {}", i),
            amount: 100 * (i as i64),
        };

        let envelope = EventEnvelope::new(
            format!("ar.batch.event.{}", i),
            "1.0.0".to_string(),
            "tenant-batch".to_string(),
            "batch".to_string(),
            format!("batch-{}", i),
            format!("trace-batch-{}", i),
            event_data,
        );

        event_ids.push(envelope.event_id);

        enqueue_event(&pool, &envelope)
            .await
            .expect("Should enqueue event");
    }

    // Verify all events are in outbox
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) as count
        FROM events_outbox
        WHERE published_at IS NULL
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("Should query outbox");

    assert_eq!(row.0, 5, "Should have 5 unpublished events in outbox");

    // Clean up
    for event_id in event_ids {
        sqlx::query("DELETE FROM events_outbox WHERE event_id = $1")
            .bind(event_id)
            .execute(&pool)
            .await
            .ok();
    }

    common::teardown_pool(pool).await;
}

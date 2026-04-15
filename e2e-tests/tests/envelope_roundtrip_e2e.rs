//! E2E Test: Envelope NATS Roundtrip (bd-2qph)
//!
//! **Phase 16: Envelope Metadata Preservation**
//!
//! ## Test Coverage
//! 1. **Envelope Creation**: Create event with full envelope metadata
//! 2. **Outbox Enqueue**: Envelope stored in outbox with all metadata fields
//! 3. **NATS Publish**: Outbox publisher emits event to NATS
//! 4. **NATS Subscribe**: Consumer receives event from NATS
//! 5. **Metadata Preservation**: All envelope fields survive roundtrip unchanged
//!
//! ## Architecture
//! - modules/subscriptions/src/outbox.rs: enqueue_event() stores envelope
//! - modules/subscriptions/src/publisher.rs: Publishes envelope to NATS
//! - NATS broker: Delivers event to subscribers
//! - Test verifies: trace_id, schema_version, mutation_class, replay_safe preserved
//!
//! ## Invariant
//! Envelope metadata survives publish-subscribe delivery unchanged.
//! Failure mode: metadata lost in transit or remapped incorrectly.

mod common;

use anyhow::Result;
use common::{setup_nats_client, subscribe_to_events};
use event_bus::EventEnvelope;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TestRoundtripEvent {
    message: String,
    sequence: i32,
}

/// Helper: Get Subscriptions database pool (delegates to common helper with retry logic)
async fn get_subscriptions_pool() -> PgPool {
    common::get_subscriptions_pool().await
}

/// Helper: Generate unique test tenant ID
fn generate_test_tenant(prefix: &str) -> String {
    format!("test-tenant-{}-{}", prefix, Uuid::new_v4())
}

/// Helper: Cleanup test data for a tenant
async fn cleanup_tenant_data(pool: &PgPool, tenant_id: &str) -> Result<()> {
    // Clean outbox
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    // Clean subscription invoice attempts
    sqlx::query("DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Helper: Manually trigger outbox publishing (for test determinism)
async fn trigger_outbox_publish(pool: &PgPool, bus: &async_nats::Client) -> Result<usize> {
    use subscriptions_rs::outbox::{fetch_unpublished_events, mark_as_published};

    let events = fetch_unpublished_events(pool, 100).await?;
    let mut published_count = 0;

    for event in events {
        let event_id = event.id;
        let nats_subject = format!("subscriptions.events.{}", event.subject);
        let payload_bytes = serde_json::to_vec(&event.payload)?;

        bus.publish(nats_subject, payload_bytes.into()).await?;
        mark_as_published(pool, event_id).await?;
        published_count += 1;
    }

    Ok(published_count)
}

/// Test that envelope metadata survives NATS roundtrip
#[tokio::test]
#[serial]
async fn test_envelope_nats_roundtrip_preserves_metadata() -> Result<()> {
    let test_id = "roundtrip_metadata";
    let tenant_id = generate_test_tenant(test_id);

    let subscriptions_pool = get_subscriptions_pool().await;
    let nats_client = setup_nats_client().await;

    // Clean up tenant data before test
    cleanup_tenant_data(&subscriptions_pool, &tenant_id).await?;

    // Step 1: Subscribe to NATS subject BEFORE publishing
    let nats_subject = "subscriptions.events.test.roundtrip";
    let mut subscriber = subscribe_to_events(&nats_client, nats_subject).await;

    // Step 2: Create envelope with ALL metadata fields populated
    let trace_id = format!("trace-{}", Uuid::new_v4());
    let correlation_id = format!("corr-{}", Uuid::new_v4());
    let causation_id = format!("cause-{}", Uuid::new_v4());

    let envelope = EventEnvelope::new(
        tenant_id.clone(),
        "subscriptions".to_string(),
        "test.roundtrip".to_string(),
        TestRoundtripEvent {
            message: "Envelope roundtrip test".to_string(),
            sequence: 42,
        },
    )
    .with_trace_id(Some(trace_id.clone()))
    .with_correlation_id(Some(correlation_id.clone()))
    .with_causation_id(Some(causation_id.clone()))
    .with_replay_safe(true)
    .with_mutation_class(Some("DATA_MUTATION".to_string()));

    // Capture original envelope metadata for comparison
    let original_event_type = envelope.event_type.clone();
    let original_schema_version = envelope.schema_version.clone();
    let original_source_module = envelope.source_module.clone();
    let original_source_version = envelope.source_version.clone();
    let original_occurred_at = envelope.occurred_at;
    let original_replay_safe = envelope.replay_safe;

    // Step 3: Enqueue event into outbox
    subscriptions_rs::outbox::enqueue_event(&subscriptions_pool, "test.roundtrip", &envelope)
        .await?;

    println!("✅ Envelope enqueued into outbox");

    // Step 4: Manually trigger outbox publish (for test determinism)
    let published_count = trigger_outbox_publish(&subscriptions_pool, &nats_client).await?;
    assert!(
        published_count > 0,
        "Expected at least 1 event to be published, got {}",
        published_count
    );

    println!(
        "✅ Outbox publisher emitted {} events to NATS",
        published_count
    );

    // Step 5: Receive event from NATS subscriber
    let received_message = timeout(Duration::from_secs(5), subscriber.next())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for NATS message"))?
        .ok_or_else(|| anyhow::anyhow!("NATS subscriber closed unexpectedly"))?;

    println!("✅ Received event from NATS");

    // Step 6: Deserialize envelope from NATS message
    let received_envelope: EventEnvelope<TestRoundtripEvent> =
        serde_json::from_slice(&received_message.payload)?;

    println!("✅ Deserialized envelope from NATS payload");

    // Step 7: Verify envelope metadata preserved
    assert_eq!(received_envelope.tenant_id, tenant_id, "tenant_id mismatch");
    assert_eq!(
        received_envelope.event_type, original_event_type,
        "event_type mismatch"
    );
    assert_eq!(
        received_envelope.schema_version, original_schema_version,
        "schema_version mismatch"
    );
    assert_eq!(
        received_envelope.source_module, original_source_module,
        "source_module mismatch"
    );
    assert_eq!(
        received_envelope.source_version, original_source_version,
        "source_version mismatch"
    );
    assert_eq!(
        received_envelope.occurred_at, original_occurred_at,
        "occurred_at mismatch"
    );
    assert_eq!(
        received_envelope.replay_safe, original_replay_safe,
        "replay_safe mismatch"
    );

    // Step 8: Verify optional metadata fields preserved
    assert_eq!(
        received_envelope.trace_id,
        Some(trace_id.clone()),
        "trace_id mismatch"
    );
    assert_eq!(
        received_envelope.correlation_id,
        Some(correlation_id.clone()),
        "correlation_id mismatch"
    );
    assert_eq!(
        received_envelope.causation_id,
        Some(causation_id.clone()),
        "causation_id mismatch"
    );
    assert_eq!(
        received_envelope.mutation_class,
        Some("DATA_MUTATION".to_string()),
        "mutation_class mismatch"
    );

    // Step 9: Verify payload preserved
    assert_eq!(
        received_envelope.payload.message, "Envelope roundtrip test",
        "payload.message mismatch"
    );
    assert_eq!(
        received_envelope.payload.sequence, 42,
        "payload.sequence mismatch"
    );

    // Clean up
    cleanup_tenant_data(&subscriptions_pool, &tenant_id).await?;

    println!("✅ All envelope metadata fields preserved through NATS roundtrip");
    println!("   - trace_id: {}", trace_id);
    println!("   - correlation_id: {}", correlation_id);
    println!("   - causation_id: {}", causation_id);
    println!("   - mutation_class: DATA_MUTATION");
    println!("   - schema_version: {}", original_schema_version);
    println!("   - replay_safe: {}", original_replay_safe);

    Ok(())
}

/// Test that multiple events preserve their distinct metadata
#[tokio::test]
#[serial]
async fn test_envelope_nats_roundtrip_multiple_events_distinct_metadata() -> Result<()> {
    let test_id = "roundtrip_multiple";
    let tenant_id = generate_test_tenant(test_id);

    let subscriptions_pool = get_subscriptions_pool().await;
    let nats_client = setup_nats_client().await;

    // Clean up tenant data before test
    cleanup_tenant_data(&subscriptions_pool, &tenant_id).await?;

    // Step 1: Subscribe to NATS subject BEFORE publishing
    let nats_subject = "subscriptions.events.test.multi";
    let mut subscriber = subscribe_to_events(&nats_client, nats_subject).await;

    // Step 2: Create 3 events with distinct metadata
    let trace_ids: Vec<String> = (1..=3).map(|i| format!("trace-multi-{}", i)).collect();
    let mutation_classes = vec!["DATA_MUTATION", "SIDE_EFFECT", "LIFECYCLE"];

    for i in 0..3_usize {
        let envelope = EventEnvelope::new(
            tenant_id.clone(),
            "subscriptions".to_string(),
            "test.multi".to_string(),
            TestRoundtripEvent {
                message: format!("Event {}", i + 1),
                sequence: (i + 1) as i32,
            },
        )
        .with_trace_id(Some(trace_ids[i].clone()))
        .with_mutation_class(Some(mutation_classes[i].to_string()));

        subscriptions_rs::outbox::enqueue_event(&subscriptions_pool, "test.multi", &envelope)
            .await?;
    }

    println!("✅ Enqueued 3 events with distinct metadata");

    // Step 3: Publish events
    let published_count = trigger_outbox_publish(&subscriptions_pool, &nats_client).await?;
    assert!(
        published_count >= 3,
        "Expected at least 3 events published, got {}",
        published_count
    );

    println!(
        "✅ Published {} events to NATS (expected ≥3)",
        published_count
    );

    // Step 4: Receive and verify all 3 events
    for i in 0..3 {
        let received_message = timeout(Duration::from_secs(5), subscriber.next())
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for NATS message {}", i + 1))?
            .ok_or_else(|| anyhow::anyhow!("NATS subscriber closed unexpectedly"))?;

        let received_envelope: EventEnvelope<TestRoundtripEvent> =
            serde_json::from_slice(&received_message.payload)?;

        // Verify this event's metadata matches one of the sent events
        assert!(
            trace_ids.contains(&received_envelope.trace_id.clone().unwrap()),
            "Unexpected trace_id: {:?}",
            received_envelope.trace_id
        );

        assert!(
            mutation_classes.contains(&received_envelope.mutation_class.as_ref().unwrap().as_str()),
            "Unexpected mutation_class: {:?}",
            received_envelope.mutation_class
        );

        println!(
            "✅ Event {} received with trace_id={:?}, mutation_class={:?}",
            i + 1,
            received_envelope.trace_id,
            received_envelope.mutation_class
        );
    }

    // Clean up
    cleanup_tenant_data(&subscriptions_pool, &tenant_id).await?;

    println!("✅ All 3 events preserved distinct metadata through NATS roundtrip");

    Ok(())
}

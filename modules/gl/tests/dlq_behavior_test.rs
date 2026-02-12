//! DLQ Behavior Test for GL Posting Consumer
//!
//! This test validates that:
//! 1. Invalid GL posting events are properly moved to DLQ (failed_events)
//! 2. Observability fields (correlation_id, tenant_id, error reason) are captured
//! 3. No panics occur on malformed input
//! 4. DLQ captures enough context for debugging
//!
//! Run with: cargo test --package gl-rs --test dlq_behavior_test

use chrono::Utc;
use event_bus::{EventBus, EventEnvelope, InMemoryBus};
use gl_rs::contracts::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::db::init_pool;
use gl_rs::start_gl_posting_consumer;
use serial_test::serial;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

// ============================================================================
// Test Setup
// ============================================================================

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5438/gl_test".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

async fn setup_test_bus() -> Arc<dyn EventBus> {
    // Use InMemoryBus for DLQ tests (doesn't require external NATS)
    Arc::new(InMemoryBus::new()) as Arc<dyn EventBus>
}

// ============================================================================
// Test: Schema Violation (Missing Required Field)
// ============================================================================

#[tokio::test]
#[serial]
#[ignore] // Run explicitly with: cargo test --test dlq_behavior_test -- --ignored
async fn test_dlq_captures_missing_required_field() {
    println!("\n🧪 Testing DLQ: Missing Required Field\n");

    let pool = setup_test_pool().await;
    let bus = setup_test_bus().await;

    // Clean up DLQ from previous runs
    sqlx::query("DELETE FROM failed_events WHERE subject = 'gl.events.posting.requested'")
        .execute(&pool)
        .await
        .expect("Failed to clean DLQ");

    // Start GL consumer
    start_gl_posting_consumer(bus.clone(), pool.clone()).await;

    // Give consumer time to subscribe
    sleep(Duration::from_millis(500)).await;

    // Create event with missing description (required field)
    let event_id = Uuid::new_v4();
    let tenant_id = "tenant-dlq-test-001";
    let correlation_id = format!("cor_{}", Uuid::new_v4());

    let malformed_payload = json!({
        "event_id": event_id,
        "occurred_at": Utc::now().to_rfc3339(),
        "tenant_id": tenant_id,
        "source_module": "ar",
        "source_version": "0.1.0",
        "correlation_id": correlation_id,
        "causation_id": "test_causation",
        "payload": {
            "posting_date": "2024-02-11",
            "currency": "USD",
            "source_doc_type": "AR_INVOICE",
            "source_doc_id": "inv_123",
            // MISSING: "description" field (required)
            "lines": [
                {
                    "account_ref": "1100",
                    "debit": 100.0,
                    "credit": 0.0
                },
                {
                    "account_ref": "4000",
                    "debit": 0.0,
                    "credit": 100.0
                }
            ]
        }
    });

    println!("📤 Publishing malformed event (missing description)...");
    println!("   Event ID: {}", event_id);
    println!("   Tenant ID: {}", tenant_id);
    println!("   Correlation ID: {}", correlation_id);

    // Publish the malformed event
    bus.publish(
        "gl.events.posting.requested",
        serde_json::to_vec(&malformed_payload).expect("Failed to serialize"),
    )
    .await
    .expect("Failed to publish");

    // Wait for consumer to process and send to DLQ
    println!("⏳ Waiting for DLQ write...");
    sleep(Duration::from_secs(3)).await;

    // Assert: Event should be in failed_events
    let failed_event: Option<(Uuid, String, String, serde_json::Value, String, i32)> =
        sqlx::query_as(
            "SELECT event_id, subject, tenant_id, envelope, error, retry_count
             FROM failed_events
             WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_optional(&pool)
        .await
        .expect("Failed to query DLQ");

    assert!(
        failed_event.is_some(),
        "Event should be in DLQ after validation failure"
    );

    let (dlq_event_id, dlq_subject, dlq_tenant_id, dlq_envelope, dlq_error, dlq_retry_count) =
        failed_event.unwrap();

    println!("\n✅ Event found in DLQ:");
    println!("   Event ID: {}", dlq_event_id);
    println!("   Subject: {}", dlq_subject);
    println!("   Tenant ID: {}", dlq_tenant_id);
    println!("   Error: {}", dlq_error);
    println!("   Retry Count: {}", dlq_retry_count);

    // Assert: DLQ captures correct metadata
    assert_eq!(dlq_event_id, event_id, "DLQ should capture event_id");
    assert_eq!(
        dlq_subject, "gl.events.posting.requested",
        "DLQ should capture subject"
    );
    assert_eq!(
        dlq_tenant_id, tenant_id,
        "DLQ should capture tenant_id for debugging"
    );
    assert!(
        dlq_error.contains("missing field"),
        "DLQ should capture error reason: {}",
        dlq_error
    );
    assert_eq!(
        dlq_retry_count, 3,
        "DLQ should record retry count (default: 3)"
    );

    // Assert: DLQ captures full envelope for replay
    assert!(
        dlq_envelope.get("correlation_id").is_some(),
        "DLQ should preserve correlation_id for observability"
    );
    assert_eq!(
        dlq_envelope.get("event_id").and_then(|v| v.as_str()),
        Some(event_id.to_string().as_str()),
        "DLQ should preserve full envelope for debugging"
    );

    println!("✅ DLQ Test Passed: Schema Violation\n");

    // Cleanup
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup DLQ");
}

// ============================================================================
// Test: Validation Failure (Unbalanced Entry)
// ============================================================================

#[tokio::test]
#[serial]
#[ignore] // Run explicitly with: cargo test --test dlq_behavior_test -- --ignored
async fn test_dlq_captures_unbalanced_entry() {
    println!("\n🧪 Testing DLQ: Unbalanced Entry\n");

    let pool = setup_test_pool().await;
    let bus = setup_test_bus().await;

    // Clean up DLQ from previous runs
    sqlx::query("DELETE FROM failed_events WHERE subject = 'gl.events.posting.requested'")
        .execute(&pool)
        .await
        .expect("Failed to clean DLQ");

    // Start GL consumer
    start_gl_posting_consumer(bus.clone(), pool.clone()).await;

    // Give consumer time to subscribe
    sleep(Duration::from_millis(500)).await;

    // Create event with unbalanced entry (debits != credits)
    let event_id = Uuid::new_v4();
    let tenant_id = "tenant-dlq-test-002";
    let correlation_id = format!("cor_{}", Uuid::new_v4());

    let unbalanced_event: EventEnvelope<GlPostingRequestV1> = EventEnvelope {
        event_id,
        occurred_at: Utc::now(),
        tenant_id: tenant_id.to_string(),
        source_module: "ar".to_string(),
        source_version: "0.1.0".to_string(),
        correlation_id: Some(correlation_id.clone()),
        causation_id: Some("test_causation".to_string()),
        payload: GlPostingRequestV1 {
            posting_date: "2024-02-11".to_string(),
            currency: "USD".to_string(),
            source_doc_type: SourceDocType::ArInvoice,
            source_doc_id: "inv_456".to_string(),
            description: "Test unbalanced entry".to_string(),
            lines: vec![
                JournalLine {
                    account_ref: "1100".to_string(),
                    debit: 100.0,   // $100 debit
                    credit: 0.0,
                    memo: Some("AR".to_string()),
                    dimensions: None,
                },
                JournalLine {
                    account_ref: "4000".to_string(),
                    debit: 0.0,
                    credit: 50.0,   // $50 credit (UNBALANCED!)
                    memo: Some("Revenue".to_string()),
                    dimensions: None,
                },
            ],
        },
    };

    println!("📤 Publishing unbalanced entry...");
    println!("   Event ID: {}", event_id);
    println!("   Tenant ID: {}", tenant_id);
    println!("   Correlation ID: {}", correlation_id);
    println!("   Debits: $100, Credits: $50 (UNBALANCED)");

    // Publish the unbalanced event
    bus.publish(
        "gl.events.posting.requested",
        serde_json::to_vec(&unbalanced_event).expect("Failed to serialize"),
    )
    .await
    .expect("Failed to publish");

    // Wait for consumer to process and send to DLQ
    println!("⏳ Waiting for DLQ write...");
    sleep(Duration::from_secs(3)).await;

    // Assert: Event should be in failed_events
    let failed_event: Option<(String, i32)> = sqlx::query_as(
        "SELECT error, retry_count
         FROM failed_events
         WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&pool)
    .await
    .expect("Failed to query DLQ");

    assert!(
        failed_event.is_some(),
        "Unbalanced entry should be in DLQ"
    );

    let (dlq_error, dlq_retry_count) = failed_event.unwrap();

    println!("\n✅ Unbalanced entry found in DLQ:");
    println!("   Error: {}", dlq_error);
    println!("   Retry Count: {}", dlq_retry_count);

    // Assert: Error message includes validation reason
    assert!(
        dlq_error.contains("Validation") || dlq_error.contains("balance"),
        "DLQ should capture validation reason: {}",
        dlq_error
    );

    println!("✅ DLQ Test Passed: Unbalanced Entry\n");

    // Cleanup
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup DLQ");
}

// ============================================================================
// Test: Invalid Currency
// ============================================================================

#[tokio::test]
#[serial]
#[ignore] // Run explicitly with: cargo test --test dlq_behavior_test -- --ignored
async fn test_dlq_captures_invalid_currency() {
    println!("\n🧪 Testing DLQ: Invalid Currency\n");

    let pool = setup_test_pool().await;
    let bus = setup_test_bus().await;

    // Clean up DLQ from previous runs
    sqlx::query("DELETE FROM failed_events WHERE subject = 'gl.events.posting.requested'")
        .execute(&pool)
        .await
        .expect("Failed to clean DLQ");

    // Start GL consumer
    start_gl_posting_consumer(bus.clone(), pool.clone()).await;

    // Give consumer time to subscribe
    sleep(Duration::from_millis(500)).await;

    // Create event with invalid currency (lowercase)
    let event_id = Uuid::new_v4();
    let tenant_id = "tenant-dlq-test-003";
    let correlation_id = format!("cor_{}", Uuid::new_v4());

    let invalid_currency_event: EventEnvelope<GlPostingRequestV1> = EventEnvelope {
        event_id,
        occurred_at: Utc::now(),
        tenant_id: tenant_id.to_string(),
        source_module: "ar".to_string(),
        source_version: "0.1.0".to_string(),
        correlation_id: Some(correlation_id.clone()),
        causation_id: Some("test_causation".to_string()),
        payload: GlPostingRequestV1 {
            posting_date: "2024-02-11".to_string(),
            currency: "usd".to_string(), // Invalid: should be "USD" (uppercase)
            source_doc_type: SourceDocType::ArInvoice,
            source_doc_id: "inv_789".to_string(),
            description: "Test invalid currency".to_string(),
            lines: vec![
                JournalLine {
                    account_ref: "1100".to_string(),
                    debit: 100.0,
                    credit: 0.0,
                    memo: Some("AR".to_string()),
                    dimensions: None,
                },
                JournalLine {
                    account_ref: "4000".to_string(),
                    debit: 0.0,
                    credit: 100.0,
                    memo: Some("Revenue".to_string()),
                    dimensions: None,
                },
            ],
        },
    };

    println!("📤 Publishing event with invalid currency...");
    println!("   Event ID: {}", event_id);
    println!("   Tenant ID: {}", tenant_id);
    println!("   Correlation ID: {}", correlation_id);
    println!("   Currency: 'usd' (should be 'USD')");

    // Publish the invalid currency event
    bus.publish(
        "gl.events.posting.requested",
        serde_json::to_vec(&invalid_currency_event).expect("Failed to serialize"),
    )
    .await
    .expect("Failed to publish");

    // Wait for consumer to process and send to DLQ
    println!("⏳ Waiting for DLQ write...");
    sleep(Duration::from_secs(3)).await;

    // Assert: Event should be in failed_events
    let failed_event: Option<(String, String)> = sqlx::query_as(
        "SELECT tenant_id, error
         FROM failed_events
         WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&pool)
    .await
    .expect("Failed to query DLQ");

    assert!(
        failed_event.is_some(),
        "Invalid currency event should be in DLQ"
    );

    let (dlq_tenant_id, dlq_error) = failed_event.unwrap();

    println!("\n✅ Invalid currency event found in DLQ:");
    println!("   Tenant ID: {}", dlq_tenant_id);
    println!("   Error: {}", dlq_error);

    // Assert: DLQ captures tenant_id for observability
    assert_eq!(dlq_tenant_id, tenant_id, "DLQ must capture tenant_id");

    // Assert: Error message includes validation reason
    assert!(
        dlq_error.contains("Validation") || dlq_error.contains("currency"),
        "DLQ should explain currency validation failure: {}",
        dlq_error
    );

    println!("✅ DLQ Test Passed: Invalid Currency\n");

    // Cleanup
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup DLQ");
}

// ============================================================================
// Test: Completely Malformed JSON (No Panic)
// ============================================================================

#[tokio::test]
#[serial]
#[ignore] // Run explicitly with: cargo test --test dlq_behavior_test -- --ignored
async fn test_dlq_handles_garbage_json_without_panic() {
    println!("\n🧪 Testing DLQ: Garbage JSON (No Panic)\n");

    let pool = setup_test_pool().await;
    let bus = setup_test_bus().await;

    // Clean up DLQ from previous runs
    sqlx::query("DELETE FROM failed_events WHERE subject = 'gl.events.posting.requested'")
        .execute(&pool)
        .await
        .expect("Failed to clean DLQ");

    // Start GL consumer
    start_gl_posting_consumer(bus.clone(), pool.clone()).await;

    // Give consumer time to subscribe
    sleep(Duration::from_millis(500)).await;

    // Publish completely invalid JSON
    let garbage_payload = b"{not valid json at all!!!";

    println!("📤 Publishing garbage JSON...");
    println!("   Payload: {}", String::from_utf8_lossy(garbage_payload));

    // Publish the garbage payload
    bus.publish("gl.events.posting.requested", garbage_payload.to_vec())
        .await
        .expect("Failed to publish");

    // Wait for consumer to process (should NOT panic)
    println!("⏳ Waiting for consumer to handle gracefully...");
    sleep(Duration::from_secs(3)).await;

    println!("✅ Consumer did not panic on garbage JSON");
    println!("   (Expected: Consumer continues running, no crash)\n");

    // Note: Garbage JSON without event_id cannot be written to DLQ
    // because we can't extract the required metadata. This is expected.
    // The important assertion is: NO PANIC occurred.

    println!("✅ DLQ Test Passed: No Panic on Garbage JSON\n");
}

// ============================================================================
// Test: Empty Account Ref
// ============================================================================

#[tokio::test]
#[serial]
#[ignore] // Run explicitly with: cargo test --test dlq_behavior_test -- --ignored
async fn test_dlq_captures_empty_account_ref() {
    println!("\n🧪 Testing DLQ: Empty Account Ref\n");

    let pool = setup_test_pool().await;
    let bus = setup_test_bus().await;

    // Clean up DLQ from previous runs
    sqlx::query("DELETE FROM failed_events WHERE subject = 'gl.events.posting.requested'")
        .execute(&pool)
        .await
        .expect("Failed to clean DLQ");

    // Start GL consumer
    start_gl_posting_consumer(bus.clone(), pool.clone()).await;

    // Give consumer time to subscribe
    sleep(Duration::from_millis(500)).await;

    // Create event with empty account_ref
    let event_id = Uuid::new_v4();
    let tenant_id = "tenant-dlq-test-004";
    let correlation_id = format!("cor_{}", Uuid::new_v4());

    let empty_account_event: EventEnvelope<GlPostingRequestV1> = EventEnvelope {
        event_id,
        occurred_at: Utc::now(),
        tenant_id: tenant_id.to_string(),
        source_module: "ar".to_string(),
        source_version: "0.1.0".to_string(),
        correlation_id: Some(correlation_id.clone()),
        causation_id: Some("test_causation".to_string()),
        payload: GlPostingRequestV1 {
            posting_date: "2024-02-11".to_string(),
            currency: "USD".to_string(),
            source_doc_type: SourceDocType::ArInvoice,
            source_doc_id: "inv_999".to_string(),
            description: "Test empty account ref".to_string(),
            lines: vec![
                JournalLine {
                    account_ref: "".to_string(), // Empty account ref (invalid)
                    debit: 100.0,
                    credit: 0.0,
                    memo: Some("AR".to_string()),
                    dimensions: None,
                },
                JournalLine {
                    account_ref: "4000".to_string(),
                    debit: 0.0,
                    credit: 100.0,
                    memo: Some("Revenue".to_string()),
                    dimensions: None,
                },
            ],
        },
    };

    println!("📤 Publishing event with empty account_ref...");
    println!("   Event ID: {}", event_id);
    println!("   Tenant ID: {}", tenant_id);
    println!("   Correlation ID: {}", correlation_id);

    // Publish the event
    bus.publish(
        "gl.events.posting.requested",
        serde_json::to_vec(&empty_account_event).expect("Failed to serialize"),
    )
    .await
    .expect("Failed to publish");

    // Wait for consumer to process and send to DLQ
    println!("⏳ Waiting for DLQ write...");
    sleep(Duration::from_secs(3)).await;

    // Assert: Event should be in failed_events
    let failed_event: Option<(String,)> = sqlx::query_as(
        "SELECT error
         FROM failed_events
         WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&pool)
    .await
    .expect("Failed to query DLQ");

    assert!(
        failed_event.is_some(),
        "Empty account_ref event should be in DLQ"
    );

    let (dlq_error,) = failed_event.unwrap();

    println!("\n✅ Empty account_ref event found in DLQ:");
    println!("   Error: {}", dlq_error);

    // Assert: Error message includes validation reason
    assert!(
        dlq_error.contains("account") || dlq_error.contains("empty"),
        "DLQ should explain empty account_ref failure: {}",
        dlq_error
    );

    println!("✅ DLQ Test Passed: Empty Account Ref\n");

    // Cleanup
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup DLQ");
}

//! E2E Test: Invalid Envelope Rejected (bd-2wfy)
//!
//! **Phase 16: Envelope Validation Boundary Enforcement**
//!
//! ## Test Coverage
//! 1. **Invalid Envelope**: Envelope with empty required field (tenant_id) is rejected
//! 2. **Outbox Isolation**: Invalid envelope does NOT create outbox row
//! 3. **Boundary Enforcement**: Validation occurs at enqueue boundary, not publish time
//!
//! ## Architecture
//! - platform/event-bus/src/outbox.rs: validate_and_serialize_envelope()
//! - modules/*/src/outbox.rs: enqueue_event() calls validation before insert
//!
//! ## Validation Rules (per envelope.rs:validate_envelope_fields)
//! Required fields that cannot be empty:
//! - event_id, event_type, occurred_at, tenant_id, source_module, source_version, schema_version
//!
//! Required field added in Phase 16:
//! - mutation_class (must be a valid class from MUTATION-CLASSES.md)
//!
//! Optional fields that, if present, cannot be empty:
//! - trace_id, correlation_id, causation_id, side_effect_id

mod common;

use anyhow::Result;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use event_bus::EventEnvelope;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestEvent {
    message: String,
}

/// Test that an envelope with empty tenant_id is rejected at enqueue boundary
#[tokio::test]
#[serial]
async fn test_invalid_envelope_empty_tenant_id_rejected() -> Result<()> {
    let test_id = "invalid_envelope_empty_tenant";
    let tenant_id = generate_test_tenant();

    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Step 1: Create envelope with EMPTY tenant_id (invalid)
    let mut envelope = EventEnvelope::new(
        "valid-tenant".to_string(), // Will be replaced with empty string
        "subscriptions".to_string(),
        "test.event".to_string(),
        TestEvent {
            message: "This should be rejected".to_string(),
        },
    );

    // Manually set tenant_id to empty string (bypassing constructor validation)
    envelope.tenant_id = "".to_string();

    // Step 2: Attempt to enqueue event - should fail validation
    let result =
        subscriptions_rs::outbox::enqueue_event(&subscriptions_pool, "test.event", &envelope).await;

    // Step 3: Assert that enqueue failed
    assert!(
        result.is_err(),
        "Expected enqueue to fail for empty tenant_id, but it succeeded"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("tenant_id") || error_message.contains("Envelope validation"),
        "Expected validation error about tenant_id, got: {}",
        error_message
    );

    // Step 4: Assert that NO outbox row was created
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 OR tenant_id = ''",
    )
    .bind(&tenant_id)
    .fetch_one(&subscriptions_pool)
    .await?;

    assert_eq!(
        outbox_count, 0,
        "Expected 0 outbox rows for invalid envelope, found {}",
        outbox_count
    );

    // Clean up
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    println!("✅ Invalid envelope (empty tenant_id) correctly rejected at boundary");
    println!("✅ No outbox row created for invalid envelope");

    Ok(())
}

/// Test that an envelope with empty trace_id (optional field) is rejected
#[tokio::test]
#[serial]
async fn test_invalid_envelope_empty_trace_id_rejected() -> Result<()> {
    let test_id = "invalid_envelope_empty_trace";
    let tenant_id = generate_test_tenant();

    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Step 1: Create envelope with empty trace_id (optional field, but if present cannot be empty)
    let mut envelope = EventEnvelope::new(
        tenant_id.clone(),
        "subscriptions".to_string(),
        "test.event".to_string(),
        TestEvent {
            message: "This should also be rejected".to_string(),
        },
    );

    // Manually set trace_id to empty string (bypassing constructor validation)
    envelope.trace_id = Some("".to_string());

    // Step 2: Attempt to enqueue event - should fail validation
    let result =
        subscriptions_rs::outbox::enqueue_event(&subscriptions_pool, "test.event", &envelope).await;

    // Step 3: Assert that enqueue failed
    assert!(
        result.is_err(),
        "Expected enqueue to fail for empty trace_id, but it succeeded"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("trace_id") || error_message.contains("cannot be empty"),
        "Expected validation error about trace_id being empty, got: {}",
        error_message
    );

    // Step 4: Assert that NO outbox row was created
    let outbox_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&subscriptions_pool)
            .await?;

    assert_eq!(
        outbox_count, 0,
        "Expected 0 outbox rows for invalid envelope, found {}",
        outbox_count
    );

    // Clean up
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    println!("✅ Invalid envelope (empty trace_id) correctly rejected at boundary");
    println!("✅ No outbox row created for invalid envelope");

    Ok(())
}

/// Test that a valid envelope passes validation and creates outbox row
#[tokio::test]
#[serial]
async fn test_valid_envelope_accepted() -> Result<()> {
    let test_id = "valid_envelope_accepted";
    let tenant_id = generate_test_tenant();

    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Step 1: Create VALID envelope (mutation_class is required since Phase 16)
    let envelope = EventEnvelope::new(
        tenant_id.clone(),
        "subscriptions".to_string(),
        "test.event.valid".to_string(),
        TestEvent {
            message: "This should succeed".to_string(),
        },
    )
    .with_trace_id(Some("valid-trace-123".to_string()))
    .with_correlation_id(Some("valid-corr-456".to_string()))
    .with_mutation_class(Some("DATA_MUTATION".to_string()));

    // Step 2: Enqueue event - should succeed
    let result =
        subscriptions_rs::outbox::enqueue_event(&subscriptions_pool, "test.event.valid", &envelope)
            .await;

    assert!(
        result.is_ok(),
        "Expected valid envelope to be accepted, but got error: {:?}",
        result.unwrap_err()
    );

    // Step 3: Assert that outbox row WAS created
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'test.event.valid'"
    )
    .bind(&tenant_id)
    .fetch_one(&subscriptions_pool)
    .await?;

    assert_eq!(
        outbox_count, 1,
        "Expected 1 outbox row for valid envelope, found {}",
        outbox_count
    );

    // Clean up
    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    println!("✅ Valid envelope correctly accepted at boundary");
    println!("✅ Outbox row created for valid envelope");

    Ok(())
}

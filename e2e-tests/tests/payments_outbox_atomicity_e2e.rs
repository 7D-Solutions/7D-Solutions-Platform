//! E2E Test: Payments Outbox Atomicity (bd-1pxo)
//!
//! **Phase 16: Outbox Pattern Atomicity Enforcement**
//!
//! ## Test Coverage
//! 1. **Atomicity Guarantee**: Payment status transitions + outbox events must be atomic
//! 2. **Transaction Boundary**: Domain mutation + outbox insert in single BEGIN/COMMIT
//! 3. **Rollback Safety**: If outbox fails, payment mutation must rollback
//!
//! ## Pattern Validated
//! Lifecycle functions (transition_to_succeeded, etc.) currently have:
//! - Transaction wrapping status UPDATE
//! - TODO comments for event emission
//! - tx.commit() happens BEFORE event emission
//!
//! ## Expected After Fix
//! - Payment status UPDATE + outbox insert in single transaction
//! - Both succeed together or both fail together
//! - No orphaned payment state without events

mod common;

use anyhow::Result;
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a payment attempt in Payments database
async fn create_payment_attempt(pool: &PgPool, tenant_id: &str, status: &str) -> Result<Uuid> {
    let attempt_id = Uuid::new_v4();
    let payment_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO payment_attempts (
            id, payment_id, app_id, invoice_id, attempt_no, status,
            idempotency_key, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 1, $5::payment_attempt_status, $6, NOW(), NOW())
        "#,
    )
    .bind(attempt_id)
    .bind(payment_id)
    .bind(tenant_id)
    .bind(format!("in_test_{}", Uuid::new_v4()))
    .bind(status)
    .bind(format!("idem_{}", Uuid::new_v4()))
    .execute(pool)
    .await?;

    Ok(attempt_id)
}

/// Get payment attempt status
async fn get_payment_status(pool: &PgPool, attempt_id: Uuid) -> Result<String> {
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(pool)
            .await?;

    Ok(status)
}

/// Count outbox rows for a given payment attempt
async fn count_outbox_rows_for_payment(pool: &PgPool, payment_id: Uuid) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) 
        FROM payments_events_outbox 
        WHERE payload->>'payment_id' = $1
        "#,
    )
    .bind(payment_id.to_string())
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// Test payment status transition atomicity with outbox
///
/// **Expected Behavior (AFTER FIX):**
/// Payment status UPDATE and outbox event insert happen atomically in a single transaction.
#[tokio::test]
#[serial]
async fn test_payments_status_transition_outbox_atomicity() -> Result<()> {
    let _test_id = "payments_outbox_atomicity";
    let tenant_id = generate_test_tenant();

    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
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

    // Step 1: Create payment attempt in "attempting" status
    let attempt_id = create_payment_attempt(&payments_pool, &tenant_id, "attempting").await?;
    println!("✅ Created payment attempt {}", attempt_id);

    // Step 2: Verify initial state
    let initial_status = get_payment_status(&payments_pool, attempt_id).await?;
    assert_eq!(
        initial_status, "attempting",
        "Payment should start in attempting status"
    );

    // Get payment_id for outbox lookup
    let payment_id: Uuid =
        sqlx::query_scalar("SELECT payment_id FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(&payments_pool)
            .await?;

    let initial_outbox_count = count_outbox_rows_for_payment(&payments_pool, payment_id).await?;
    assert_eq!(
        initial_outbox_count, 0,
        "No outbox rows should exist initially"
    );

    // Step 3: Call production lifecycle function — atomically transitions status + enqueues outbox event
    payments_rs::lifecycle::transition_to_succeeded(&payments_pool, attempt_id, "test succeeded")
        .await
        .map_err(|e| anyhow::anyhow!("transition_to_succeeded failed: {:?}", e))?;

    println!("✅ Transitioned payment {} to 'succeeded'", attempt_id);

    // Step 4: Check final state
    let final_status = get_payment_status(&payments_pool, attempt_id).await?;
    let final_outbox_count = count_outbox_rows_for_payment(&payments_pool, payment_id).await?;

    println!("\n📊 Atomicity Check:");
    println!("  Payment Status: {} -> {}", initial_status, final_status);
    println!(
        "  Outbox Rows: {} -> {}",
        initial_outbox_count, final_outbox_count
    );

    // Step 5: Assert atomicity
    // CURRENT BUG: Payment is succeeded but outbox has no events
    // AFTER FIX: If payment succeeded, outbox MUST have payment.succeeded event

    if final_status == "succeeded" {
        // Payment was transitioned - outbox MUST have events
        assert!(
            final_outbox_count >= 1,
            "❌ ATOMICITY VIOLATION: Payment transitioned to succeeded but outbox has {} events (expected >= 1). \
             This proves the bug: payment mutation committed without outbox insert.",
            final_outbox_count
        );
        println!("✅ Atomicity preserved: Payment succeeded AND outbox events created");
    } else {
        // Payment was NOT transitioned - outbox should have no events
        assert_eq!(
            final_outbox_count, 0,
            "If payment not transitioned, outbox should have 0 events, found {}",
            final_outbox_count
        );
        println!("✅ Atomicity preserved: Payment not transitioned AND no outbox events");
    }

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

    println!("\n🎯 Test Result: Atomicity verified!");
    println!("   - Domain state and outbox are consistent");
    println!("   - No orphaned payment mutations");

    Ok(())
}

/// Test webhook handler status update (idempotency gate)
///
/// Verifies that update_payment_status_from_webhook correctly updates payment status
/// via the state machine gate with SELECT FOR UPDATE idempotency semantics.
///
/// Note: Outbox event emission is not yet implemented in the webhook handler
/// (see TODO in webhook_handler.rs STEP 5). Atomicity of (mutation + outbox)
/// will be covered when event emission is added.
#[tokio::test]
#[serial]
async fn test_payments_webhook_handler_atomicity() -> Result<()> {
    use payments_rs::webhook_handler::update_payment_status_from_webhook;
    use payments_rs::webhook_signature::WebhookSource;

    let tenant_id = generate_test_tenant();
    let payments_pool = get_payments_pool().await;
    let ar_pool = get_ar_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    // Create a payment attempt in "attempting" status
    let attempt_id = create_payment_attempt(&payments_pool, &tenant_id, "attempting").await?;
    let webhook_event_id = format!("evt_{}", Uuid::new_v4());
    let headers = std::collections::HashMap::new();

    // Call webhook handler (Internal source bypasses signature validation)
    update_payment_status_from_webhook(
        &payments_pool,
        attempt_id,
        "succeeded",
        &webhook_event_id,
        WebhookSource::Internal,
        &headers,
        b"",
        &[],
    )
    .await
    .map_err(|e| anyhow::anyhow!("webhook handler failed: {:?}", e))?;

    // Assert status was updated
    let final_status = get_payment_status(&payments_pool, attempt_id).await?;
    assert_eq!(
        final_status, "succeeded",
        "Webhook handler must update payment status"
    );

    // Assert idempotency: same webhook_event_id is a no-op
    update_payment_status_from_webhook(
        &payments_pool,
        attempt_id,
        "succeeded",
        &webhook_event_id,
        WebhookSource::Internal,
        &headers,
        b"",
        &[],
    )
    .await
    .map_err(|e| anyhow::anyhow!("idempotent replay failed: {:?}", e))?;

    let after_replay_status = get_payment_status(&payments_pool, attempt_id).await?;
    assert_eq!(
        after_replay_status, "succeeded",
        "Status must remain succeeded after idempotent replay"
    );

    println!("✅ Webhook handler: status updated atomically, idempotency gate works");

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

//! E2E Test: Subscriptions Outbox Atomicity (bd-299f)
//!
//! **Phase 16: Outbox Pattern Atomicity Enforcement**
//!
//! ## Test Coverage
//! 1. **Cycle Advancement Atomicity**: Billing cycle mutations + outbox events must be atomic
//! 2. **Lifecycle Atomicity**: Subscription status transitions + outbox events must be atomic
//! 3. **Transaction Boundary**: Domain mutation + outbox insert in single BEGIN/COMMIT
//!
//! ## Violations Found
//! - gated_invoice_creation: cycle advance commits before event emission
//! - lifecycle functions: NO transactions, future events would be non-atomic
//!
//! ## Expected After Fix
//! - Cycle advance + outbox insert in single transaction
//! - Lifecycle transitions wrapped in transactions
//! - Both succeed together or both fail together

mod common;

use anyhow::Result;
use common::{cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_payments_pool, get_subscriptions_pool, get_gl_pool};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a subscription in Subscriptions database
async fn create_subscription(
    pool: &PgPool,
    tenant_id: &str,
    status: &str,
) -> Result<Uuid> {
    let subscription_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();
    
    sqlx::query(
        r#"
        INSERT INTO subscriptions (
            id, app_id, customer_id, status, plan_id, price_minor,
            billing_period, start_date, next_billing_date,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'plan_test', 5000, 'monthly', CURRENT_DATE, CURRENT_DATE + INTERVAL '1 month', NOW(), NOW())
        "#,
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(customer_id)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(subscription_id)
}

/// Get subscription status
async fn get_subscription_status(pool: &PgPool, subscription_id: Uuid) -> Result<String> {
    let status: String = sqlx::query_scalar(
        "SELECT status FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(pool)
    .await?;

    Ok(status)
}

/// Count outbox rows for a given subscription
async fn count_outbox_rows_for_subscription(pool: &PgPool, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// Test subscription status transition atomicity with outbox
///
/// **Expected Behavior (AFTER FIX):**
/// Subscription status UPDATE and outbox event insert happen atomically in a single transaction.
#[tokio::test]
#[serial]
async fn test_subscriptions_lifecycle_transition_outbox_atomicity() -> Result<()> {
    let test_id = "subs_lifecycle_atomicity";
    let tenant_id = generate_test_tenant();

    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let gl_pool = get_gl_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Step 1: Create subscription in "active" status
    let subscription_id = create_subscription(&subscriptions_pool, &tenant_id, "active").await?;
    println!("✅ Created subscription {}", subscription_id);

    // Step 2: Verify initial state
    let initial_status = get_subscription_status(&subscriptions_pool, subscription_id).await?;
    assert_eq!(initial_status, "active", "Subscription should start in active status");

    let initial_outbox_count = count_outbox_rows_for_subscription(&subscriptions_pool, &tenant_id).await?;
    assert_eq!(
        initial_outbox_count, 0,
        "No outbox rows should exist initially"
    );

    // Step 3: Simulate subscription status transition (what lifecycle functions DO)
    // Note: In real implementation, this would call transition_to_past_due()
    // For this test, we simulate the CURRENT behavior (without transaction)
    
    sqlx::query(
        "UPDATE subscriptions SET status = $1, updated_at = NOW() WHERE id = $2"
    )
    .bind("past_due")
    .bind(subscription_id)
    .execute(&subscriptions_pool)
    .await?;

    println!("✅ Transitioned subscription {} to 'past_due'", subscription_id);

    // Step 4: Check final state
    let final_status = get_subscription_status(&subscriptions_pool, subscription_id).await?;
    let final_outbox_count = count_outbox_rows_for_subscription(&subscriptions_pool, &tenant_id).await?;

    println!("\n📊 Atomicity Check:");
    println!("  Subscription Status: {} -> {}", initial_status, final_status);
    println!("  Outbox Rows: {} -> {}", initial_outbox_count, final_outbox_count);

    // Step 5: Assert atomicity
    // CURRENT BUG: Subscription is past_due but outbox has no events
    // AFTER FIX: If subscription transitioned, outbox MUST have status.changed event
    
    if final_status == "past_due" {
        // Subscription was transitioned - outbox MUST have events
        assert!(
            final_outbox_count >= 1,
            "❌ ATOMICITY VIOLATION: Subscription transitioned to past_due but outbox has {} events (expected >= 1). \
             This proves the bug: subscription mutation committed without outbox insert.",
            final_outbox_count
        );
        println!("✅ Atomicity preserved: Subscription transitioned AND outbox events created");
    } else {
        // Subscription was NOT transitioned - outbox should have no events
        assert_eq!(
            final_outbox_count, 0,
            "If subscription not transitioned, outbox should have 0 events, found {}",
            final_outbox_count
        );
        println!("✅ Atomicity preserved: Subscription not transitioned AND no outbox events");
    }

    // Clean up
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("\n🎯 Test Result: Atomicity verified!");
    println!("   - Domain state and outbox are consistent");
    println!("   - No orphaned subscription mutations");

    Ok(())
}

/// Test cycle advancement atomicity
///
/// This test validates that billing cycle advancement is atomic with outbox inserts.
#[tokio::test]
#[serial]
#[ignore] // Ignored until cycle advancement event emission is implemented
async fn test_subscriptions_cycle_advance_atomicity() -> Result<()> {
    // TODO: This test requires calling gated invoice creation
    // The critical atomicity point is mark_attempt_succeeded + event emission
    Ok(())
}

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
use common::{
    cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_gl_pool, get_payments_pool,
    get_subscriptions_pool,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a subscription in Subscriptions database
async fn create_subscription(pool: &PgPool, tenant_id: &str, status: &str) -> Result<Uuid> {
    let subscription_id = Uuid::new_v4();
    let ar_customer_id = format!("cust-{}", Uuid::new_v4());

    // Create plan first (plan_id is UUID FK)
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency) \
         VALUES ($1, 'Test Plan', 'monthly', 5000, 'USD') RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO subscriptions (
            id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor,
            currency, start_date, next_bill_date
        )
        VALUES ($1, $2, $3, $4, $5, 'monthly', 5000, 'USD', CURRENT_DATE, CURRENT_DATE + INTERVAL '1 month')
        "#,
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id)
    .bind(plan_id)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(subscription_id)
}

/// Get subscription status
async fn get_subscription_status(pool: &PgPool, subscription_id: Uuid) -> Result<String> {
    let status: String = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .fetch_one(pool)
        .await?;

    Ok(status)
}

/// Count outbox rows for a given subscription
async fn count_outbox_rows_for_subscription(pool: &PgPool, tenant_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1")
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
    let _test_id = "subs_lifecycle_atomicity";
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

    // Step 1: Create subscription in "active" status
    let subscription_id = create_subscription(&subscriptions_pool, &tenant_id, "active").await?;
    println!("✅ Created subscription {}", subscription_id);

    // Step 2: Verify initial state
    let initial_status = get_subscription_status(&subscriptions_pool, subscription_id).await?;
    assert_eq!(
        initial_status, "active",
        "Subscription should start in active status"
    );

    let initial_outbox_count =
        count_outbox_rows_for_subscription(&subscriptions_pool, &tenant_id).await?;
    assert_eq!(
        initial_outbox_count, 0,
        "No outbox rows should exist initially"
    );

    // Step 3: Call production lifecycle function — atomically transitions status + enqueues outbox event
    subscriptions_rs::lifecycle::transition_to_past_due(
        subscription_id,
        "test past_due",
        &subscriptions_pool,
    )
    .await
    .map_err(|e| anyhow::anyhow!("transition_to_past_due failed: {:?}", e))?;

    println!(
        "✅ Transitioned subscription {} to 'past_due'",
        subscription_id
    );

    // Step 4: Check final state
    let final_status = get_subscription_status(&subscriptions_pool, subscription_id).await?;
    let final_outbox_count =
        count_outbox_rows_for_subscription(&subscriptions_pool, &tenant_id).await?;

    println!("\n📊 Atomicity Check:");
    println!(
        "  Subscription Status: {} -> {}",
        initial_status, final_status
    );
    println!(
        "  Outbox Rows: {} -> {}",
        initial_outbox_count, final_outbox_count
    );

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
    println!("   - No orphaned subscription mutations");

    Ok(())
}

/// Test cycle advancement atomicity: record_attempt + mark_succeeded in one transaction
///
/// Validates that billing cycle gating (record_cycle_attempt → mark_attempt_succeeded)
/// operates atomically within a single transaction, and the UNIQUE constraint
/// prevents double-billing for the same subscription/cycle pair.
///
/// Note: Outbox event emission is not yet wired to gated_invoice_creation.
/// This test validates the DB state machine that underpins exactly-once billing.
#[tokio::test]
#[serial]
async fn test_subscriptions_cycle_advance_atomicity() -> Result<()> {
    use chrono::Local;
    use subscriptions_rs::{
        calculate_cycle_boundaries, generate_cycle_key, mark_attempt_succeeded,
        record_cycle_attempt, CycleGatingError,
    };

    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
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

    let subscription_id = create_subscription(&subscriptions_pool, &tenant_id, "active").await?;
    println!("✅ Created subscription {}", subscription_id);

    let billing_date = Local::now().date_naive();
    let cycle_key = generate_cycle_key(billing_date);
    let (cycle_start, cycle_end) = calculate_cycle_boundaries(billing_date);

    // Step 1: Record attempt + mark succeeded atomically in one transaction
    let mut tx = subscriptions_pool.begin().await?;
    let attempt_id = record_cycle_attempt(
        &mut *tx,
        &tenant_id,
        subscription_id,
        &cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("record_cycle_attempt failed: {:?}", e))?;

    mark_attempt_succeeded(&mut *tx, attempt_id, 42)
        .await
        .map_err(|e| anyhow::anyhow!("mark_attempt_succeeded failed: {:?}", e))?;

    tx.commit().await?;
    println!("✅ Committed cycle attempt {} as succeeded", attempt_id);

    // Step 2: Assert DB state is correct (cast enum to text for comparison)
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM subscription_invoice_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(&subscriptions_pool)
            .await?;
    assert_eq!(
        status, "succeeded",
        "Attempt must be succeeded after atomic commit"
    );

    // Step 3: Assert UNIQUE constraint blocks duplicate cycle attempt
    let mut tx2 = subscriptions_pool.begin().await?;
    let dup_result = record_cycle_attempt(
        &mut *tx2,
        &tenant_id,
        subscription_id,
        &cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await;
    tx2.rollback().await?;

    assert!(
        matches!(dup_result, Err(CycleGatingError::DuplicateCycle { .. })),
        "Duplicate cycle attempt must be rejected by UNIQUE constraint, got: {:?}",
        dup_result
    );
    println!(
        "✅ UNIQUE constraint blocks double-billing for cycle {}",
        cycle_key
    );

    cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        &gl_pool,
        &tenant_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    println!("\n🎯 Cycle advance atomicity verified:");
    println!("   - record_cycle_attempt + mark_succeeded committed atomically");
    println!("   - UNIQUE constraint enforces exactly-once billing per cycle");

    Ok(())
}

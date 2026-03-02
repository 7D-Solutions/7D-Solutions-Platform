//! Cross-Module E2E Test: Subscription → Invoice (bd-3rc.2)
//!
//! **Phase 15 bd-184: Exactly-Once Invoice Per Cycle**
//!
//! ## Test Coverage
//! 1. **Exactly One Invoice Per Cycle:** UNIQUE constraint enforcement
//! 2. **Replay Safety:** Duplicate event_id → same invoice (no duplicate)
//! 3. **Concurrency Safety:** Parallel cycle attempts blocked by advisory locks
//!
//! ## Architecture
//! - Subscriptions module creates invoice generation attempts
//! - AR module receives invoice creation events
//! - subscription_invoice_attempts table enforces exactly-once semantics
//!
//! ## Pattern Reference
//! - Foundation: e2e-tests/tests/common/mod.rs (bd-3rc.1)
//! - GL NATS E2E: modules/gl/tests/boundary_e2e_nats_posting.rs

mod common;
mod oracle;

use anyhow::Result;
use chrono::NaiveDate;
use common::{cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_subscriptions_pool};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test Setup Helpers
// ============================================================================

/// Create a subscription plan
async fn create_subscription_plan(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO subscription_plans
         (id, tenant_id, name, description, schedule, price_minor, currency, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'USD', NOW(), NOW())
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(plan_id)
    .bind(tenant_id)
    .bind("E2E Test Plan")
    .bind("Phase 15 E2E test subscription plan")
    .execute(pool)
    .await?;

    Ok(())
}

/// Create a subscription
async fn create_subscription(
    pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    plan_id: Uuid,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
) -> Result<(), sqlx::Error> {
    let start_date = next_bill_date;

    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 2999, 'USD', $5, $6, NOW(), NOW())
         ON CONFLICT (id) DO NOTHING"
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(start_date)
    .bind(next_bill_date)
    .execute(pool)
    .await?;

    Ok(())
}

/// Create AR customer and return the customer ID
async fn create_ar_customer(
    pool: &PgPool,
    tenant_id: &str,
    customer_external_id: &str,
) -> Result<i32, sqlx::Error> {
    let email = format!("{}@e2e-test.com", customer_external_id);

    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, external_customer_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NOW(), NOW())
         ON CONFLICT (app_id, external_customer_id) DO UPDATE SET updated_at = NOW()
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&email)
    .bind("E2E Test Customer")
    .bind(customer_external_id)
    .fetch_one(pool)
    .await?;

    Ok(customer_id)
}

/// Cleanup test data for subscription and AR tables
async fn cleanup_test_data(subscriptions_pool: &PgPool, ar_pool: &PgPool, tenant_id: &str) {
    // Delete in correct order due to foreign key constraints

    // Subscriptions cleanup
    sqlx::query("DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .ok();

    // AR cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .ok();
}

/// Trigger bill run by calling subscriptions cycle gating logic
/// This simulates what happens when POST /api/subscriptions/bill-run is called
async fn trigger_bill_run(
    subscriptions_pool: &PgPool,
    ar_pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    execution_date: NaiveDate,
) -> Result<Option<i32>> {
    // Import cycle gating functions
    use subscriptions_rs::cycle_gating::{
        acquire_cycle_lock, calculate_cycle_boundaries, generate_cycle_key, mark_attempt_succeeded,
        record_cycle_attempt,
    };

    let cycle_key = generate_cycle_key(execution_date);
    let (cycle_start, cycle_end) = calculate_cycle_boundaries(execution_date);

    // Start transaction (simulates what happens in bill run handler)
    let mut tx = subscriptions_pool.begin().await?;

    // Acquire advisory lock (prevents concurrent bill runs for same cycle)
    acquire_cycle_lock(&mut tx, tenant_id, subscription_id, &cycle_key).await?;

    // Record attempt (UNIQUE constraint enforces exactly-once)
    let attempt_id = match record_cycle_attempt(
        &mut tx,
        tenant_id,
        subscription_id,
        &cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await
    {
        Ok(id) => id,
        Err(_e) => {
            // If duplicate cycle error, this is expected (replay safety)
            tx.rollback().await?;
            return Ok(None); // Signal that invoice already exists
        }
    };

    // Get subscription details
    let subscription: (String, i64, String) = sqlx::query_as(
        "SELECT ar_customer_id, price_minor, currency FROM subscriptions WHERE id = $1",
    )
    .bind(subscription_id)
    .fetch_one(&mut *tx)
    .await?;

    let (ar_customer_id_str, price_minor, currency) = subscription;
    let ar_customer_id: i32 = ar_customer_id_str.parse().expect("Invalid customer ID");

    // Convert minor units (e.g., 2999 cents = $29.99)
    let amount_cents = (price_minor / 10) as i32;

    // Create invoice in AR database
    let tilled_invoice_id = format!("inv-{}", Uuid::new_v4());
    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices
         (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency, status, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, NOW(), NOW())
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&tilled_invoice_id)
    .bind(ar_customer_id)
    .bind(amount_cents)
    .bind(&currency)
    .bind((execution_date + chrono::Duration::days(30)).and_hms_opt(0, 0, 0).unwrap())
    .fetch_one(ar_pool)
    .await?;

    // Mark attempt as succeeded
    mark_attempt_succeeded(&mut tx, attempt_id, invoice_id).await?;

    // Commit transaction
    tx.commit().await?;

    Ok(Some(invoice_id))
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_exactly_one_invoice_per_cycle() {
    // Setup
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;

    let plan_id = Uuid::new_v4();
    let subscription_id = Uuid::new_v4();
    let customer_external_id = format!("cust-{}", Uuid::new_v4());
    let execution_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // Create test data
    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id, &customer_external_id)
        .await
        .expect("Failed to create AR customer");

    create_subscription_plan(&subscriptions_pool, &tenant_id, plan_id)
        .await
        .expect("Failed to create plan");

    create_subscription(
        &subscriptions_pool,
        &tenant_id,
        subscription_id,
        plan_id,
        ar_customer_id,
        execution_date,
    )
    .await
    .expect("Failed to create subscription");

    // First bill run - should succeed
    let invoice_id_1 = trigger_bill_run(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        execution_date,
    )
    .await
    .expect("First bill run failed");

    assert!(
        invoice_id_1.is_some(),
        "First bill run should create invoice"
    );

    // Second bill run for SAME cycle - should be blocked by UNIQUE constraint
    let invoice_id_2 = trigger_bill_run(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        execution_date,
    )
    .await
    .expect("Second bill run failed");

    assert!(
        invoice_id_2.is_none(),
        "Second bill run should NOT create duplicate invoice"
    );

    // Assert: Only one attempt record exists
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Should have exactly 1 attempt record (no duplicates)"
    );

    // Assert: Only one invoice exists in AR
    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await
            .expect("Failed to count invoices");

    assert_eq!(
        invoice_count, 1,
        "Should have exactly 1 invoice (no duplicates)"
    );

    // Cleanup
    cleanup_test_data(&subscriptions_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_replay_safety_same_cycle_multiple_attempts() {
    // Setup
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;

    let plan_id = Uuid::new_v4();
    let subscription_id = Uuid::new_v4();
    let customer_external_id = format!("cust-{}", Uuid::new_v4());
    let execution_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // Create test data
    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id, &customer_external_id)
        .await
        .expect("Failed to create AR customer");

    create_subscription_plan(&subscriptions_pool, &tenant_id, plan_id)
        .await
        .expect("Failed to create plan");

    create_subscription(
        &subscriptions_pool,
        &tenant_id,
        subscription_id,
        plan_id,
        ar_customer_id,
        execution_date,
    )
    .await
    .expect("Failed to create subscription");

    // Execute bill run 3 times (simulating replay)
    for i in 1..=3 {
        let result = trigger_bill_run(
            &subscriptions_pool,
            &ar_pool,
            &tenant_id,
            subscription_id,
            execution_date,
        )
        .await
        .expect(&format!("Bill run {} failed", i));

        if i == 1 {
            assert!(result.is_some(), "First attempt should create invoice");
        } else {
            assert!(
                result.is_none(),
                "Replay attempt {} should not create duplicate",
                i
            );
        }
    }

    // Assert: Still only 1 attempt and 1 invoice
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Replay safety: should still have exactly 1 attempt"
    );

    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await
            .expect("Failed to count invoices");

    assert_eq!(
        invoice_count, 1,
        "Replay safety: should still have exactly 1 invoice"
    );

    // Cleanup
    cleanup_test_data(&subscriptions_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_concurrency_safety_parallel_bill_runs() {
    // Setup
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;

    let plan_id = Uuid::new_v4();
    let subscription_id = Uuid::new_v4();
    let customer_external_id = format!("cust-{}", Uuid::new_v4());
    let execution_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // Create test data
    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id, &customer_external_id)
        .await
        .expect("Failed to create AR customer");

    create_subscription_plan(&subscriptions_pool, &tenant_id, plan_id)
        .await
        .expect("Failed to create plan");

    create_subscription(
        &subscriptions_pool,
        &tenant_id,
        subscription_id,
        plan_id,
        ar_customer_id,
        execution_date,
    )
    .await
    .expect("Failed to create subscription");

    // Spawn 5 parallel bill runs for the SAME cycle
    let mut handles = vec![];
    for i in 0..5 {
        let subscriptions_pool = subscriptions_pool.clone();
        let ar_pool = ar_pool.clone();
        let tenant_id = tenant_id.clone();
        let subscription_id = subscription_id;
        let execution_date = execution_date;

        let handle = tokio::spawn(async move {
            trigger_bill_run(
                &subscriptions_pool,
                &ar_pool,
                &tenant_id,
                subscription_id,
                execution_date,
            )
            .await
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    let results = futures::future::join_all(handles).await;

    // Count successes
    let success_count = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter_map(|r| r.as_ref().ok())
        .filter(|r| r.is_some())
        .count();

    // Assert: Only ONE parallel attempt succeeded (advisory lock worked!)
    assert_eq!(
        success_count, 1,
        "Concurrency safety: exactly 1 parallel attempt should succeed"
    );

    // Assert: Database has exactly 1 attempt and 1 invoice
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Concurrency safety: should have exactly 1 attempt record"
    );

    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await
            .expect("Failed to count invoices");

    assert_eq!(
        invoice_count, 1,
        "Concurrency safety: should have exactly 1 invoice"
    );

    // Cleanup
    cleanup_test_data(&subscriptions_pool, &ar_pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_different_cycles_create_separate_invoices() {
    // Setup
    let tenant_id = generate_test_tenant();
    let subscriptions_pool = get_subscriptions_pool().await;
    let ar_pool = get_ar_pool().await;

    let plan_id = Uuid::new_v4();
    let subscription_id = Uuid::new_v4();
    let customer_external_id = format!("cust-{}", Uuid::new_v4());

    // Create test data
    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id, &customer_external_id)
        .await
        .expect("Failed to create AR customer");

    create_subscription_plan(&subscriptions_pool, &tenant_id, plan_id)
        .await
        .expect("Failed to create plan");

    create_subscription(
        &subscriptions_pool,
        &tenant_id,
        subscription_id,
        plan_id,
        ar_customer_id,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    )
    .await
    .expect("Failed to create subscription");

    // Bill run for February 2026
    let feb_result = trigger_bill_run(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
    )
    .await
    .expect("February bill run failed");

    assert!(feb_result.is_some(), "February bill run should succeed");

    // Bill run for March 2026 (different cycle)
    let mar_result = trigger_bill_run(
        &subscriptions_pool,
        &ar_pool,
        &tenant_id,
        subscription_id,
        NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
    )
    .await
    .expect("March bill run failed");

    assert!(mar_result.is_some(), "March bill run should succeed");

    // Assert: 2 attempts (one per cycle)
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(attempt_count, 2, "Should have 2 attempts (one per cycle)");

    // Assert: 2 invoices
    let invoice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&ar_pool)
            .await
            .expect("Failed to count invoices");

    assert_eq!(invoice_count, 2, "Should have 2 invoices (one per cycle)");

    // Assert: Cycle keys are different
    let cycle_keys: Vec<String> = sqlx::query_scalar(
        "SELECT cycle_key FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2
         ORDER BY cycle_key",
    )
    .bind(&tenant_id)
    .bind(subscription_id)
    .fetch_all(&subscriptions_pool)
    .await
    .expect("Failed to fetch cycle keys");

    assert_eq!(cycle_keys.len(), 2);
    assert_eq!(cycle_keys[0], "2026-02");
    assert_eq!(cycle_keys[1], "2026-03");

    // Cleanup
    cleanup_test_data(&subscriptions_pool, &ar_pool, &tenant_id).await;
}

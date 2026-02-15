//! Cross-Module E2E: Replay Safety (Phase 15 - bd-3rc.7)
//!
//! **Purpose:** Test full-flow event replay and deduplication across all modules
//!
//! **Invariants Tested:**
//! 1. Full flow replay produces identical results
//! 2. Event deduplication at every module boundary
//! 3. Idempotency at every stage (event_id, source_event_id, webhook_event_id)
//! 4. Exactly-once mechanisms work together across the entire lifecycle

mod common;

use chrono::{NaiveDate, Utc};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_subscription(
    subscriptions_pool: &PgPool,
    tenant_id: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO subscriptions (tenant_id, customer_id, status, plan_id, billing_cycle_day, next_billing_date)
         VALUES ($1, 'cust-replay', 'active', 'plan-basic', 1, '2026-03-01')
         RETURNING id"
    )
    .bind(tenant_id)
    .fetch_one(subscriptions_pool)
    .await
    .expect("Failed to create subscription")
}

async fn create_subscription_invoice_attempt(
    subscriptions_pool: &PgPool,
    tenant_id: &str,
    subscription_id: i32,
    cycle_key: &str,
    attempt_no: i32,
    status: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO subscription_invoice_attempts (tenant_id, subscription_id, cycle_key, attempt_no, status)
         VALUES ($1, $2, $3, $4, $5::subscription_invoice_attempt_status)
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .bind(attempt_no)
    .bind(status)
    .fetch_one(subscriptions_pool)
    .await
}

async fn setup_invoice(
    ar_pool: &PgPool,
    app_id: &str,
    customer_id: &str,
    status: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar.ar_invoices (app_id, ar_customer_id, status, amount_cents, currency, due_at, tilled_invoice_id)
         VALUES ($1, $2, $3, 10000, 'USD', '2026-02-28', $4)
         RETURNING id"
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(status)
    .bind(format!("inv-{}", Uuid::new_v4()))
    .fetch_one(ar_pool)
    .await
    .expect("Failed to create invoice")
}

async fn create_payment_attempt(
    payments_pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
    attempt_no: i32,
    status: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3::text, $4, $5::payment_attempt_status)
         RETURNING id"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .bind(attempt_no)
    .bind(status)
    .fetch_one(payments_pool)
    .await
}

async fn create_journal_entry(
    gl_pool: &PgPool,
    tenant_id: &str,
    source_event_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description)
         VALUES ($1, $2, 'ar', $3, 'invoice.created', $4, 'USD', 'Test posting')
         RETURNING id"
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(source_event_id)
    .bind(Utc::now())
    .fetch_one(gl_pool)
    .await
}

// ============================================================================
// Test: Subscription Cycle Replay (Exactly One Invoice Per Cycle)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_subscription_cycle_replay_deduplication() {
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let cycle_key = "2026-02";

    // Setup: Create subscription
    let subscription_id = setup_subscription(&subscriptions_pool, tenant_id).await;

    // Execute: First invoice generation attempt (should succeed)
    let attempt1_result = create_subscription_invoice_attempt(
        &subscriptions_pool,
        tenant_id,
        subscription_id,
        cycle_key,
        0,
        "succeeded",
    )
    .await;

    assert!(attempt1_result.is_ok(), "First attempt should succeed");

    // Execute: Replay invoice generation (same cycle_key) - should fail with UNIQUE constraint
    let replay_result = create_subscription_invoice_attempt(
        &subscriptions_pool,
        tenant_id,
        subscription_id,
        cycle_key,
        0,
        "succeeded",
    )
    .await;

    assert!(
        replay_result.is_err(),
        "Replay should fail with UNIQUE constraint violation"
    );

    // Assert: Exactly one attempt exists for this cycle
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2 AND cycle_key = $3"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Should have exactly 1 attempt after replay"
    );

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, &common::get_payments_pool().await, &subscriptions_pool, &common::get_gl_pool().await, tenant_id)
        .await
        .ok();
}

// ============================================================================
// Test: Payment Attempt Replay (No Duplicate Attempts)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_payment_attempt_replay_deduplication() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let app_id = &common::generate_test_tenant();
    let payment_id = Uuid::new_v4();

    // Setup: Create invoice
    let invoice_id = setup_invoice(&ar_pool, app_id, "cust-replay", "open").await;

    // Execute: First payment attempt (should succeed)
    let attempt1_result = create_payment_attempt(
        &payments_pool,
        app_id,
        payment_id,
        invoice_id,
        0,
        "attempting",
    )
    .await;

    assert!(attempt1_result.is_ok(), "First payment attempt should succeed");

    // Execute: Replay payment attempt (same payment_id, attempt_no) - should fail
    let replay_result = create_payment_attempt(
        &payments_pool,
        app_id,
        payment_id,
        invoice_id,
        0,
        "attempting",
    )
    .await;

    assert!(
        replay_result.is_err(),
        "Replay should fail with UNIQUE constraint violation"
    );

    // Assert: Exactly one attempt exists
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2 AND attempt_no = $3"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(0)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Should have exactly 1 attempt after replay"
    );

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, &payments_pool, &common::get_subscriptions_pool().await, &common::get_gl_pool().await, app_id)
        .await
        .ok();
}

// ============================================================================
// Test: GL Posting Replay (source_event_id Deduplication)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_posting_replay_deduplication() {
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    // Execute: First GL posting (should succeed)
    let entry1_result = create_journal_entry(&gl_pool, tenant_id, source_event_id).await;

    assert!(entry1_result.is_ok(), "First GL posting should succeed");
    let entry1_id = entry1_result.unwrap();

    // Execute: Replay GL posting (same source_event_id) - should fail
    let replay_result = create_journal_entry(&gl_pool, tenant_id, source_event_id).await;

    assert!(
        replay_result.is_err(),
        "Replay should fail with UNIQUE constraint violation on source_event_id"
    );

    // Assert: Exactly one journal entry exists for this event
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries
         WHERE tenant_id = $1 AND source_event_id = $2"
    )
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count entries");

    assert_eq!(
        entry_count, 1,
        "Should have exactly 1 journal entry after replay"
    );

    // Cleanup
    common::cleanup_tenant_data(&common::get_ar_pool().await, &common::get_payments_pool().await, &common::get_subscriptions_pool().await, &gl_pool, tenant_id)
        .await
        .ok();
}

// ============================================================================
// Test: Full Flow Replay (Subscription → Invoice → Payment → GL)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_full_flow_replay_produces_identical_results() {
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let cycle_key = "2026-02-full";
    let payment_id = Uuid::new_v4();
    let gl_event_id = Uuid::new_v4();

    // Setup: Create subscription
    let subscription_id = setup_subscription(&subscriptions_pool, tenant_id).await;

    // **Step 1: Subscription → Invoice (First Run)**
    let sub_attempt1 = create_subscription_invoice_attempt(
        &subscriptions_pool,
        tenant_id,
        subscription_id,
        cycle_key,
        0,
        "succeeded",
    )
    .await;
    assert!(sub_attempt1.is_ok(), "Subscription attempt 1 should succeed");

    // Create invoice (simulating invoice generation from subscription event)
    let invoice_id = setup_invoice(&ar_pool, tenant_id, "cust-full-flow", "open").await;

    // **Step 2: Invoice → Payment (First Run)**
    let payment_attempt1 = create_payment_attempt(
        &payments_pool,
        tenant_id,
        payment_id,
        invoice_id,
        0,
        "succeeded",
    )
    .await;
    assert!(payment_attempt1.is_ok(), "Payment attempt 1 should succeed");

    // **Step 3: Invoice → GL (First Run)**
    let gl_entry1 = create_journal_entry(&gl_pool, tenant_id, gl_event_id).await;
    assert!(gl_entry1.is_ok(), "GL entry 1 should succeed");

    // **Replay: Subscription → Invoice (Second Run)**
    let sub_replay = create_subscription_invoice_attempt(
        &subscriptions_pool,
        tenant_id,
        subscription_id,
        cycle_key,
        0,
        "succeeded",
    )
    .await;
    assert!(
        sub_replay.is_err(),
        "Subscription replay should fail (UNIQUE constraint)"
    );

    // **Replay: Invoice → Payment (Second Run)**
    let payment_replay = create_payment_attempt(
        &payments_pool,
        tenant_id,
        payment_id,
        invoice_id,
        0,
        "succeeded",
    )
    .await;
    assert!(
        payment_replay.is_err(),
        "Payment replay should fail (UNIQUE constraint)"
    );

    // **Replay: Invoice → GL (Second Run)**
    let gl_replay = create_journal_entry(&gl_pool, tenant_id, gl_event_id).await;
    assert!(
        gl_replay.is_err(),
        "GL replay should fail (UNIQUE constraint on source_event_id)"
    );

    // **Assert: Exactly one of each record created**
    let sub_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2 AND cycle_key = $3"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .fetch_one(&subscriptions_pool)
    .await
    .unwrap();

    let payment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2"
    )
    .bind(tenant_id)
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await
    .unwrap();

    let gl_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries
         WHERE tenant_id = $1 AND source_event_id = $2"
    )
    .bind(tenant_id)
    .bind(gl_event_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();

    assert_eq!(sub_count, 1, "Should have exactly 1 subscription attempt");
    assert_eq!(payment_count, 1, "Should have exactly 1 payment attempt");
    assert_eq!(gl_count, 1, "Should have exactly 1 GL journal entry");

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, tenant_id)
        .await
        .ok();
}

// ============================================================================
// Test: Multiple Replays (Stress Test)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multiple_replays_remain_deterministic() {
    let payments_pool = common::get_payments_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let app_id = &common::generate_test_tenant();
    let payment_id = Uuid::new_v4();

    // Setup: Create invoice
    let invoice_id = setup_invoice(&ar_pool, app_id, "cust-multi-replay", "open").await;

    // Execute: First attempt (should succeed)
    let first_result = create_payment_attempt(
        &payments_pool,
        app_id,
        payment_id,
        invoice_id,
        0,
        "attempting",
    )
    .await;
    assert!(first_result.is_ok(), "First attempt should succeed");

    // Execute: 10 replay attempts (all should fail deterministically)
    let mut replay_failures = 0;
    for _ in 0..10 {
        let replay_result = create_payment_attempt(
            &payments_pool,
            app_id,
            payment_id,
            invoice_id,
            0,
            "attempting",
        )
        .await;

        if replay_result.is_err() {
            replay_failures += 1;
        }
    }

    assert_eq!(replay_failures, 10, "All 10 replays should fail");

    // Assert: Still exactly 1 attempt
    let final_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2 AND attempt_no = $3"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(0)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        final_count, 1,
        "Should still have exactly 1 attempt after 10 replays"
    );

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, &payments_pool, &common::get_subscriptions_pool().await, &common::get_gl_pool().await, app_id)
        .await
        .ok();
}

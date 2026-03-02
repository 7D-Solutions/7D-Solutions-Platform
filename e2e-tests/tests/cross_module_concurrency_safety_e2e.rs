//! Cross-Module E2E: Concurrency Safety (Phase 15 - bd-3rc.8)
//!
//! **Purpose:** Test concurrent operations across all modules with UNIQUE constraints
//!
//! **Invariants Tested:**
//! 1. Parallel subscription cycle attempts → UNIQUE constraint prevents duplicates
//! 2. Parallel payment attempts → UNIQUE constraint enforcement
//! 3. Parallel GL postings → source_event_id deduplication
//! 4. Race conditions handled gracefully (no deadlocks, no lost updates)
//!
//! **Pattern:** Deterministic barrier — pre-insert 1 known row, then prove N concurrent
//! inserts ALL fail (UNIQUE constraint). Tests the invariant, not timing-dependent success counts.

mod common;
mod oracle;

use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::task::JoinSet;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a subscription plan for testing
async fn create_plan_for_test(pool: &PgPool, tenant_id: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency) \
         VALUES ($1, 'Concurrency Test Plan', 'monthly', 5000, 'USD') RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test plan")
}

/// Create a subscription row for testing, returns subscription UUID
async fn create_subscription_for_test(pool: &PgPool, tenant_id: &str, plan_id: Uuid) -> Uuid {
    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions \
         (id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date) \
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 5000, 'USD', CURRENT_DATE, CURRENT_DATE + INTERVAL '1 month')",
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(format!("cust-{}", Uuid::new_v4()))
    .bind(plan_id)
    .execute(pool)
    .await
    .expect("Failed to create test subscription");
    subscription_id
}

/// Insert a subscription_invoice_attempt matching the actual schema.
///
/// UNIQUE key: (tenant_id, subscription_id, cycle_key)
async fn create_subscription_invoice_attempt(
    pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO subscription_invoice_attempts \
         (tenant_id, subscription_id, cycle_key, cycle_start, cycle_end, status) \
         VALUES ($1, $2, $3, '2026-02-01', '2026-02-28', 'attempting') \
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .fetch_one(pool)
    .await
}

/// Insert a payment_attempt matching the actual schema.
///
/// UNIQUE key: (app_id, payment_id, attempt_no)
async fn create_payment_attempt(
    pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
    attempt_no: i32,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status) \
         VALUES ($1, $2, $3::text, $4, 'attempting') \
         RETURNING id",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .bind(attempt_no)
    .fetch_one(pool)
    .await
}

/// Insert a journal_entry matching the actual schema.
///
/// UNIQUE key: source_event_id
async fn create_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    source_event_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries \
         (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description) \
         VALUES ($1, $2, 'ar', $3, 'invoice.created', CURRENT_TIMESTAMP, 'USD', 'Concurrent test posting') \
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(pool)
    .await
}

// ============================================================================
// Test: Parallel Subscription Cycle Attempts (UNIQUE constraint enforced)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_parallel_subscription_cycle_attempts() {
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let cycle_key = "2026-02-concurrent";

    // Setup: create plan + subscription (FK dependency for subscription_id)
    let plan_id = create_plan_for_test(&subscriptions_pool, tenant_id).await;
    let subscription_id =
        create_subscription_for_test(&subscriptions_pool, tenant_id, plan_id).await;

    let pool = Arc::new(subscriptions_pool);

    // Barrier: pre-insert 1 known row to establish the UNIQUE key
    create_subscription_invoice_attempt(pool.as_ref(), tenant_id, subscription_id, cycle_key)
        .await
        .expect("Pre-insert must succeed for fresh (tenant_id, subscription_id, cycle_key)");

    // Spawn 10 concurrent attempts with the same UNIQUE key — all must be rejected
    let mut join_set = JoinSet::new();
    for _ in 0..10 {
        let pool_clone = Arc::clone(&pool);
        let tenant_clone = tenant_id.to_string();
        let cycle_clone = cycle_key.to_string();

        join_set.spawn(async move {
            create_subscription_invoice_attempt(
                &pool_clone,
                &tenant_clone,
                subscription_id,
                &cycle_clone,
            )
            .await
        });
    }

    let mut concurrent_successes = 0;
    while let Some(result) = join_set.join_next().await {
        if matches!(result, Ok(Ok(_))) {
            concurrent_successes += 1;
        }
    }

    // Invariant: UNIQUE constraint must reject every concurrent attempt
    assert_eq!(
        concurrent_successes,
        0,
        "All concurrent inserts must fail — UNIQUE (tenant_id, subscription_id, cycle_key) enforced"
    );

    // Invariant: exactly 1 record remains (the pre-inserted barrier row)
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts \
         WHERE tenant_id = $1 AND subscription_id = $2 AND cycle_key = $3",
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Exactly 1 attempt record — UNIQUE constraint enforces at-most-once per cycle"
    );

    // Oracle: assert all module invariants hold
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: pool.as_ref(),
        gl_pool: &gl_pool,
        audit_pool: &audit_pool,
        app_id: tenant_id,
        tenant_id,
    };
    oracle::assert_cross_module_invariants(&ctx)
        .await
        .expect("Oracle invariants should pass");

    common::cleanup_tenant_data(&ar_pool, &payments_pool, pool.as_ref(), &gl_pool, tenant_id)
        .await
        .ok();
}

// ============================================================================
// Test: Parallel Payment Attempts (UNIQUE Constraint Enforcement)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_parallel_payment_attempts() {
    let payments_pool = common::get_payments_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let app_id = &common::generate_test_tenant();
    let payment_id = Uuid::new_v4();
    let invoice_id = 12345;

    let pool = Arc::new(payments_pool);

    // Barrier: pre-insert 1 known row to establish the UNIQUE key
    create_payment_attempt(pool.as_ref(), app_id, payment_id, invoice_id, 0)
        .await
        .expect("Pre-insert must succeed for fresh (app_id, payment_id, attempt_no)");

    // Spawn 10 concurrent attempts with the same UNIQUE key — all must be rejected
    let mut join_set = JoinSet::new();
    for _ in 0..10 {
        let pool_clone = Arc::clone(&pool);
        let app_id_clone = app_id.to_string();

        join_set.spawn(async move {
            create_payment_attempt(&pool_clone, &app_id_clone, payment_id, invoice_id, 0).await
        });
    }

    let mut concurrent_successes = 0;
    while let Some(result) = join_set.join_next().await {
        if matches!(result, Ok(Ok(_))) {
            concurrent_successes += 1;
        }
    }

    // Invariant: UNIQUE constraint must reject every concurrent attempt
    assert_eq!(
        concurrent_successes, 0,
        "All concurrent inserts must fail — UNIQUE (app_id, payment_id, attempt_no) enforced"
    );

    // Invariant: exactly 1 record
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts \
         WHERE app_id = $1 AND payment_id = $2 AND attempt_no = $3",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(0)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Exactly 1 payment attempt — UNIQUE constraint enforces at-most-once"
    );

    // Oracle: assert all module invariants hold
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: pool.as_ref(),
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id,
        tenant_id: app_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx)
        .await
        .expect("Oracle invariants should pass");

    common::cleanup_tenant_data(
        &ar_pool,
        pool.as_ref(),
        &subscriptions_pool,
        &gl_pool,
        app_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: Parallel GL Postings (source_event_id Deduplication)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_parallel_gl_postings() {
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    let pool = Arc::new(gl_pool);

    // Barrier: pre-insert 1 known row to establish the UNIQUE key
    create_journal_entry(pool.as_ref(), tenant_id, source_event_id)
        .await
        .expect("Pre-insert must succeed for fresh source_event_id");

    // Spawn 10 concurrent attempts with the same UNIQUE key — all must be rejected
    let mut join_set = JoinSet::new();
    for _ in 0..10 {
        let pool_clone = Arc::clone(&pool);
        let tenant_clone = tenant_id.to_string();

        join_set.spawn(async move {
            create_journal_entry(&pool_clone, &tenant_clone, source_event_id).await
        });
    }

    let mut concurrent_successes = 0;
    while let Some(result) = join_set.join_next().await {
        if matches!(result, Ok(Ok(_))) {
            concurrent_successes += 1;
        }
    }

    // Invariant: UNIQUE constraint must reject every concurrent attempt
    assert_eq!(
        concurrent_successes, 0,
        "All concurrent GL inserts must fail — UNIQUE source_event_id enforced"
    );

    // Invariant: exactly 1 record
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries \
         WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count entries");

    assert_eq!(
        entry_count, 1,
        "Exactly 1 journal entry — source_event_id deduplication enforced"
    );

    // Oracle: assert all module invariants hold
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: pool.as_ref(),
        audit_pool: &audit_pool,
        app_id: tenant_id,
        tenant_id,
    };
    oracle::assert_cross_module_invariants(&ctx)
        .await
        .expect("Oracle invariants should pass");

    common::cleanup_tenant_data(
        &ar_pool,
        &payments_pool,
        &subscriptions_pool,
        pool.as_ref(),
        tenant_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: Mixed Concurrent Operations (Subscription + Payment + GL)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_mixed_concurrent_operations() {
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let tenant_id = &common::generate_test_tenant();

    // Setup unique IDs for each module
    let cycle_key = "2026-02-mixed";
    let payment_id = Uuid::new_v4();
    let invoice_id = 11111;
    let gl_event_id = Uuid::new_v4();

    // Setup subscription FK dependency
    let plan_id = create_plan_for_test(&subscriptions_pool, tenant_id).await;
    let subscription_id =
        create_subscription_for_test(&subscriptions_pool, tenant_id, plan_id).await;

    let sub_pool = Arc::new(subscriptions_pool);
    let pay_pool = Arc::new(payments_pool);
    let gl_pool_arc = Arc::new(gl_pool);

    // Barrier: pre-insert 1 row per module (deterministic)
    create_subscription_invoice_attempt(sub_pool.as_ref(), tenant_id, subscription_id, cycle_key)
        .await
        .expect("Sub pre-insert must succeed");
    create_payment_attempt(pay_pool.as_ref(), tenant_id, payment_id, invoice_id, 0)
        .await
        .expect("Payment pre-insert must succeed");
    create_journal_entry(gl_pool_arc.as_ref(), tenant_id, gl_event_id)
        .await
        .expect("GL pre-insert must succeed");

    // Spawn 30 concurrent attempts (10 per module) — all must fail due to UNIQUE constraints
    let mut join_set = JoinSet::new();

    for _ in 0..10 {
        let pool = Arc::clone(&sub_pool);
        let tenant = tenant_id.to_string();
        let cycle = cycle_key.to_string();
        join_set.spawn(async move {
            create_subscription_invoice_attempt(&pool, &tenant, subscription_id, &cycle).await
        });
    }

    for _ in 0..10 {
        let pool = Arc::clone(&pay_pool);
        let app = tenant_id.to_string();
        join_set.spawn(async move {
            create_payment_attempt(&pool, &app, payment_id, invoice_id, 0).await
        });
    }

    for _ in 0..10 {
        let pool = Arc::clone(&gl_pool_arc);
        let tenant = tenant_id.to_string();
        join_set.spawn(async move { create_journal_entry(&pool, &tenant, gl_event_id).await });
    }

    let mut total_concurrent_successes = 0;
    while let Some(result) = join_set.join_next().await {
        if matches!(result, Ok(Ok(_))) {
            total_concurrent_successes += 1;
        }
    }

    // Invariant: all 30 concurrent inserts must fail (pre-existing rows hold UNIQUE keys)
    assert_eq!(
        total_concurrent_successes, 0,
        "All 30 concurrent inserts must fail — UNIQUE constraints enforced across all modules"
    );

    // Invariant: exactly 1 record per module
    let sub_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts \
         WHERE tenant_id = $1 AND subscription_id = $2",
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .fetch_one(sub_pool.as_ref())
    .await
    .unwrap();

    let pay_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts \
         WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(tenant_id)
    .bind(payment_id)
    .fetch_one(pay_pool.as_ref())
    .await
    .unwrap();

    let gl_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries \
         WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(tenant_id)
    .bind(gl_event_id)
    .fetch_one(gl_pool_arc.as_ref())
    .await
    .unwrap();

    assert_eq!(
        sub_count, 1,
        "Exactly 1 subscription attempt — uniqueness enforced"
    );
    assert_eq!(
        pay_count, 1,
        "Exactly 1 payment attempt — uniqueness enforced"
    );
    assert_eq!(gl_count, 1, "Exactly 1 GL entry — uniqueness enforced");

    // Oracle: assert all module invariants hold
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: pay_pool.as_ref(),
        subscriptions_pool: sub_pool.as_ref(),
        gl_pool: gl_pool_arc.as_ref(),
        audit_pool: &audit_pool,
        app_id: tenant_id,
        tenant_id,
    };
    oracle::assert_cross_module_invariants(&ctx)
        .await
        .expect("Oracle invariants should pass");

    common::cleanup_tenant_data(
        &ar_pool,
        pay_pool.as_ref(),
        sub_pool.as_ref(),
        gl_pool_arc.as_ref(),
        tenant_id,
    )
    .await
    .ok();
}

// ============================================================================
// Test: High Concurrency Stress Test (100 Concurrent Attempts)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_high_concurrency_stress() {
    let payments_pool = common::get_payments_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let app_id = &common::generate_test_tenant();
    let payment_id = Uuid::new_v4();
    let invoice_id = 99999;

    let pool = Arc::new(payments_pool);

    // Barrier: pre-insert 1 known row
    create_payment_attempt(pool.as_ref(), app_id, payment_id, invoice_id, 0)
        .await
        .expect("Pre-insert must succeed for fresh (app_id, payment_id, attempt_no)");

    // Spawn 100 concurrent attempts — all must fail due to UNIQUE constraint
    let mut join_set = JoinSet::new();
    for _ in 0..100 {
        let pool_clone = Arc::clone(&pool);
        let app_id_clone = app_id.to_string();

        join_set.spawn(async move {
            create_payment_attempt(&pool_clone, &app_id_clone, payment_id, invoice_id, 0).await
        });
    }

    let mut concurrent_successes = 0;
    while let Some(result) = join_set.join_next().await {
        if matches!(result, Ok(Ok(_))) {
            concurrent_successes += 1;
        }
    }

    // Invariant: UNIQUE constraint must reject all 100 concurrent attempts
    assert_eq!(
        concurrent_successes, 0,
        "All 100 concurrent inserts must fail — UNIQUE constraint holds under high concurrency"
    );

    // Invariant: exactly 1 record
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts \
         WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(app_id)
    .bind(payment_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Exactly 1 record after 100 concurrent attempts — at-most-once enforced"
    );

    // Oracle: assert all module invariants hold
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let audit_pool = common::get_audit_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: pool.as_ref(),
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id,
        tenant_id: app_id,
        audit_pool: &audit_pool,
    };
    oracle::assert_cross_module_invariants(&ctx)
        .await
        .expect("Oracle invariants should pass");

    common::cleanup_tenant_data(
        &ar_pool,
        pool.as_ref(),
        &subscriptions_pool,
        &gl_pool,
        app_id,
    )
    .await
    .ok();
}

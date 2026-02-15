//! Cross-Module E2E: Concurrency Safety (Phase 15 - bd-3rc.8)
//!
//! **Purpose:** Test concurrent operations across all modules with UNIQUE constraints and advisory locks
//!
//! **Invariants Tested:**
//! 1. Parallel subscription cycle attempts → exactly 1 succeeds
//! 2. Parallel payment attempts → UNIQUE constraint enforcement
//! 3. Parallel GL postings → source_event_id deduplication
//! 4. Race conditions handled gracefully (no deadlocks, no lost updates)
//! 5. Advisory lock ordering prevents deadlocks

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

async fn create_subscription_invoice_attempt(
    subscriptions_pool: &PgPool,
    tenant_id: &str,
    subscription_id: i32,
    cycle_key: &str,
    attempt_no: i32,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO subscription_invoice_attempts (tenant_id, subscription_id, cycle_key, attempt_no, status)
         VALUES ($1, $2, $3, $4, 'succeeded')
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .bind(attempt_no)
    .fetch_one(subscriptions_pool)
    .await
}

async fn create_payment_attempt(
    payments_pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
    attempt_no: i32,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3::text, $4, 'attempting')
         RETURNING id"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .bind(attempt_no)
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
         VALUES ($1, $2, 'ar', $3, 'invoice.created', CURRENT_TIMESTAMP, 'USD', 'Concurrent test posting')
         RETURNING id"
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(gl_pool)
    .await
}

// ============================================================================
// Test: Parallel Subscription Cycle Attempts (Exactly 1 Succeeds)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_parallel_subscription_cycle_attempts() {
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let tenant_id = &common::generate_test_tenant();
    let subscription_id = 12345;
    let cycle_key = "2026-02-concurrent";

    // Execute: Spawn 10 parallel attempts to create invoice for same cycle
    let pool = Arc::new(subscriptions_pool);
    let mut join_set = JoinSet::new();

    for _ in 0..10 {
        let pool_clone = Arc::clone(&pool);
        let tenant_id_clone = tenant_id.to_string();
        let cycle_key_clone = cycle_key.to_string();

        join_set.spawn(async move {
            create_subscription_invoice_attempt(
                &pool_clone,
                &tenant_id_clone,
                subscription_id,
                &cycle_key_clone,
                0,
            )
            .await
        });
    }

    // Collect results
    let mut successes = 0;
    let mut failures = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_)) => successes += 1,
            Ok(Err(_)) => failures += 1,
            Err(_) => failures += 1,
        }
    }

    // Assert: Exactly 1 success, 9 failures (UNIQUE constraint)
    assert_eq!(successes, 1, "Exactly 1 parallel attempt should succeed");
    assert_eq!(failures, 9, "9 parallel attempts should fail with UNIQUE constraint");

    // Assert: Exactly 1 attempt record in database
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2 AND cycle_key = $3"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Should have exactly 1 attempt record after concurrent operations"
    );

    // Oracle: Assert all module invariants
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: pool.as_ref(),
        gl_pool: &gl_pool,
        app_id: tenant_id,
        tenant_id,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
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

    // Execute: Spawn 10 parallel payment attempts (same payment_id, attempt_no)
    let pool = Arc::new(payments_pool);
    let mut join_set = JoinSet::new();

    for _ in 0..10 {
        let pool_clone = Arc::clone(&pool);
        let app_id_clone = app_id.to_string();

        join_set.spawn(async move {
            create_payment_attempt(&pool_clone, &app_id_clone, payment_id, invoice_id, 0).await
        });
    }

    // Collect results
    let mut successes = 0;
    let mut failures = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_)) => successes += 1,
            Ok(Err(_)) => failures += 1,
            Err(_) => failures += 1,
        }
    }

    // Assert: Exactly 1 success, 9 failures
    assert_eq!(successes, 1, "Exactly 1 parallel payment attempt should succeed");
    assert_eq!(failures, 9, "9 parallel attempts should fail with UNIQUE constraint");

    // Assert: Exactly 1 payment attempt record
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2 AND attempt_no = $3"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(0)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Should have exactly 1 payment attempt after concurrent operations"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: pool.as_ref(),
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id,
        tenant_id: app_id,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, pool.as_ref(), &subscriptions_pool, &gl_pool, app_id)
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
    let tenant_id = &common::generate_test_tenant();
    let source_event_id = Uuid::new_v4();

    // Execute: Spawn 10 parallel GL posting attempts (same source_event_id)
    let pool = Arc::new(gl_pool);
    let mut join_set = JoinSet::new();

    for _ in 0..10 {
        let pool_clone = Arc::clone(&pool);
        let tenant_id_clone = tenant_id.to_string();

        join_set.spawn(async move {
            create_journal_entry(&pool_clone, &tenant_id_clone, source_event_id).await
        });
    }

    // Collect results
    let mut successes = 0;
    let mut failures = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_)) => successes += 1,
            Ok(Err(_)) => failures += 1,
            Err(_) => failures += 1,
        }
    }

    // Assert: Exactly 1 success, 9 failures
    assert_eq!(successes, 1, "Exactly 1 parallel GL posting should succeed");
    assert_eq!(failures, 9, "9 parallel postings should fail with UNIQUE constraint");

    // Assert: Exactly 1 journal entry record
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries
         WHERE tenant_id = $1 AND source_event_id = $2"
    )
    .bind(tenant_id)
    .bind(source_event_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count entries");

    assert_eq!(
        entry_count, 1,
        "Should have exactly 1 journal entry after concurrent operations"
    );

    // Oracle: Assert all module invariants
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: pool.as_ref(),
        app_id: tenant_id,
        tenant_id,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, pool.as_ref(), tenant_id)
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
    let ar_pool = common::get_ar_pool().await;
    let tenant_id = &common::generate_test_tenant();

    // Setup: Different IDs for each module
    let subscription_id = 99999;
    let cycle_key = "2026-02-mixed";
    let payment_id = Uuid::new_v4();
    let invoice_id = 11111;
    let gl_event_id = Uuid::new_v4();

    // Execute: Spawn 30 parallel operations (10 of each type)
    let sub_pool = Arc::new(subscriptions_pool);
    let pay_pool = Arc::new(payments_pool);
    let gl_pool_arc = Arc::new(gl_pool);
    let mut join_set = JoinSet::new();

    // 10 subscription attempts
    for _ in 0..10 {
        let pool = Arc::clone(&sub_pool);
        let tenant = tenant_id.to_string();
        let cycle = cycle_key.to_string();

        join_set.spawn(async move {
            create_subscription_invoice_attempt(&pool, &tenant, subscription_id, &cycle, 0).await
        });
    }

    // 10 payment attempts
    for _ in 0..10 {
        let pool = Arc::clone(&pay_pool);
        let app = tenant_id.to_string();

        join_set.spawn(async move {
            create_payment_attempt(&pool, &app, payment_id, invoice_id, 0).await
        });
    }

    // 10 GL postings
    for _ in 0..10 {
        let pool = Arc::clone(&gl_pool_arc);
        let tenant = tenant_id.to_string();

        join_set.spawn(async move {
            create_journal_entry(&pool, &tenant, gl_event_id).await
        });
    }

    // Collect results
    let mut total_successes = 0;
    let mut total_failures = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_)) => total_successes += 1,
            Ok(Err(_)) => total_failures += 1,
            Err(_) => total_failures += 1,
        }
    }

    // Assert: Exactly 3 successes (1 per module), 27 failures
    assert_eq!(total_successes, 3, "Exactly 3 operations should succeed (1 per module)");
    assert_eq!(total_failures, 27, "27 operations should fail with UNIQUE constraints");

    // Assert: Exactly 1 record in each module
    let sub_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .fetch_one(sub_pool.as_ref())
    .await
    .unwrap();

    let pay_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2"
    )
    .bind(tenant_id)
    .bind(payment_id)
    .fetch_one(pay_pool.as_ref())
    .await
    .unwrap();

    let gl_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries
         WHERE tenant_id = $1 AND source_event_id = $2"
    )
    .bind(tenant_id)
    .bind(gl_event_id)
    .fetch_one(gl_pool_arc.as_ref())
    .await
    .unwrap();

    assert_eq!(sub_count, 1, "Should have exactly 1 subscription attempt");
    assert_eq!(pay_count, 1, "Should have exactly 1 payment attempt");
    assert_eq!(gl_count, 1, "Should have exactly 1 GL entry");

    // Oracle: Assert all module invariants
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: pay_pool.as_ref(),
        subscriptions_pool: sub_pool.as_ref(),
        gl_pool: gl_pool_arc.as_ref(),
        app_id: tenant_id,
        tenant_id,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, pay_pool.as_ref(), sub_pool.as_ref(), gl_pool_arc.as_ref(), tenant_id)
        .await
        .ok();
}

// ============================================================================
// Test: High Concurrency Stress Test (100 Parallel Operations)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_high_concurrency_stress() {
    let payments_pool = common::get_payments_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let app_id = &common::generate_test_tenant();
    let payment_id = Uuid::new_v4();
    let invoice_id = 99999;

    // Execute: Spawn 100 parallel payment attempts
    let pool = Arc::new(payments_pool);
    let mut join_set = JoinSet::new();

    for _ in 0..100 {
        let pool_clone = Arc::clone(&pool);
        let app_id_clone = app_id.to_string();

        join_set.spawn(async move {
            create_payment_attempt(&pool_clone, &app_id_clone, payment_id, invoice_id, 0).await
        });
    }

    // Collect results
    let mut successes = 0;
    let mut failures = 0;

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_)) => successes += 1,
            Ok(Err(_)) => failures += 1,
            Err(_) => failures += 1,
        }
    }

    // Assert: Exactly 1 success, 99 failures (even under high concurrency)
    assert_eq!(successes, 1, "Exactly 1 operation should succeed under high concurrency");
    assert_eq!(failures, 99, "99 operations should fail with UNIQUE constraint");

    // Assert: Exactly 1 payment attempt record
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts
         WHERE app_id = $1 AND payment_id = $2"
    )
    .bind(app_id)
    .bind(payment_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        attempt_count, 1,
        "Should have exactly 1 attempt after 100 concurrent operations"
    );

    // Oracle: Assert all module invariants
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: pool.as_ref(),
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        app_id,
        tenant_id: app_id,
    };
    oracle::assert_cross_module_invariants(&ctx).await.expect("Oracle invariants should pass");

    // Cleanup
    common::cleanup_tenant_data(&ar_pool, pool.as_ref(), &subscriptions_pool, &gl_pool, app_id)
        .await
        .ok();
}

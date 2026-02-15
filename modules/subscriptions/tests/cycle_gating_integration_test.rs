//! Integration tests for subscription cycle gating
//!
//! Tests verify:
//! 1. Exactly-once invoice per subscription cycle (UNIQUE constraint)
//! 2. Advisory locks prevent concurrent duplicate attempts
//! 3. Cycle key generation is deterministic
//! 4. Attempt ledger correctly tracks successes and failures

use chrono::NaiveDate;
use sqlx::PgPool;
use subscriptions_rs::cycle_gating::{
    acquire_cycle_lock, calculate_cycle_boundaries, cycle_attempt_exists, generate_cycle_key,
    mark_attempt_failed, mark_attempt_succeeded, record_cycle_attempt, CycleGatingError,
};
use uuid::Uuid;

// Test helper to create a test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5433/subscriptions_test".to_string()
    });

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// Test helper to create a test subscription
async fn create_test_subscription(pool: &PgPool, tenant_id: &str) -> Uuid {
    let ar_customer_id = format!("customer-{}", Uuid::new_v4());

    // Create a subscription plan first
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Test Plan', 'monthly', 2999, 'USD')
         RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test plan");

    // Create subscription
    let subscription_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscriptions (tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, 'active', 'monthly', 2999, 'USD', CURRENT_DATE, CURRENT_DATE + INTERVAL '1 month')
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&ar_customer_id)
    .bind(plan_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test subscription");

    subscription_id
}

// ============================================================================
// Cycle Key Tests
// ============================================================================

#[tokio::test]
async fn test_cycle_key_determinism() {
    let date1 = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let date2 = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let date3 = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

    let key1 = generate_cycle_key(date1);
    let key2 = generate_cycle_key(date2);
    let key3 = generate_cycle_key(date3);

    assert_eq!(key1, key2);
    assert_eq!(key2, key3);
    assert_eq!(key1, "2026-02");
}

#[tokio::test]
async fn test_cycle_boundaries() {
    let date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let (start, end) = calculate_cycle_boundaries(date);

    assert_eq!(start, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
    assert_eq!(end, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
}

// ============================================================================
// Attempt Ledger Tests
// ============================================================================

#[tokio::test]
async fn test_record_cycle_attempt() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;
    let cycle_key = "2026-02";
    let (cycle_start, cycle_end) =
        calculate_cycle_boundaries(NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());

    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    let attempt_id = record_cycle_attempt(
        &mut tx,
        tenant_id,
        subscription_id,
        cycle_key,
        cycle_start,
        cycle_end,
        Some("test-idempotency-key"),
    )
    .await
    .expect("Failed to record attempt");

    tx.commit().await.expect("Failed to commit transaction");

    // Verify attempt was recorded
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts WHERE id = $1",
    )
    .bind(attempt_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query attempts");

    assert_eq!(count, 1);

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_duplicate_cycle_attempt_fails() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;
    let cycle_key = "2026-02";
    let (cycle_start, cycle_end) =
        calculate_cycle_boundaries(NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());

    // First attempt succeeds
    let mut tx1 = pool.begin().await.expect("Failed to begin transaction");
    let _attempt_id = record_cycle_attempt(
        &mut tx1,
        tenant_id,
        subscription_id,
        cycle_key,
        cycle_start,
        cycle_end,
        Some("test-idempotency-key-1"),
    )
    .await
    .expect("First attempt should succeed");
    tx1.commit().await.expect("Failed to commit transaction");

    // Second attempt fails with DuplicateCycle error
    let mut tx2 = pool.begin().await.expect("Failed to begin transaction");
    let result = record_cycle_attempt(
        &mut tx2,
        tenant_id,
        subscription_id,
        cycle_key,
        cycle_start,
        cycle_end,
        Some("test-idempotency-key-2"),
    )
    .await;

    assert!(result.is_err());
    match result {
        Err(CycleGatingError::DuplicateCycle {
            subscription_id: sid,
            cycle_key: ck,
        }) => {
            assert_eq!(sid, subscription_id);
            assert_eq!(ck, cycle_key);
        }
        _ => panic!("Expected DuplicateCycle error"),
    }

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_cycle_attempt_exists() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;
    let cycle_key = "2026-02";
    let (cycle_start, cycle_end) =
        calculate_cycle_boundaries(NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());

    // Initially no attempt exists
    let mut tx1 = pool.begin().await.expect("Failed to begin transaction");
    let exists_before =
        cycle_attempt_exists(&mut tx1, tenant_id, subscription_id, cycle_key)
            .await
            .expect("Failed to check if attempt exists");
    tx1.rollback()
        .await
        .expect("Failed to rollback transaction");

    assert!(!exists_before);

    // Record attempt
    let mut tx2 = pool.begin().await.expect("Failed to begin transaction");
    let _attempt_id = record_cycle_attempt(
        &mut tx2,
        tenant_id,
        subscription_id,
        cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await
    .expect("Failed to record attempt");
    tx2.commit().await.expect("Failed to commit transaction");

    // Now attempt exists
    let mut tx3 = pool.begin().await.expect("Failed to begin transaction");
    let exists_after =
        cycle_attempt_exists(&mut tx3, tenant_id, subscription_id, cycle_key)
            .await
            .expect("Failed to check if attempt exists");
    tx3.rollback()
        .await
        .expect("Failed to rollback transaction");

    assert!(exists_after);

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_mark_attempt_succeeded() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;
    let cycle_key = "2026-03";
    let (cycle_start, cycle_end) =
        calculate_cycle_boundaries(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());

    // Record attempt
    let mut tx1 = pool.begin().await.expect("Failed to begin transaction");
    let attempt_id = record_cycle_attempt(
        &mut tx1,
        tenant_id,
        subscription_id,
        cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await
    .expect("Failed to record attempt");
    tx1.commit().await.expect("Failed to commit transaction");

    // Mark as succeeded
    let mut tx2 = pool.begin().await.expect("Failed to begin transaction");
    mark_attempt_succeeded(&mut tx2, attempt_id, 12345)
        .await
        .expect("Failed to mark succeeded");
    tx2.commit().await.expect("Failed to commit transaction");

    // Verify status
    let (status, ar_invoice_id): (String, Option<i32>) = sqlx::query_as(
        "SELECT status::text, ar_invoice_id FROM subscription_invoice_attempts WHERE id = $1",
    )
    .bind(attempt_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query attempt");

    assert_eq!(status, "succeeded");
    assert_eq!(ar_invoice_id, Some(12345));

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_mark_attempt_failed() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;
    let cycle_key = "2026-04";
    let (cycle_start, cycle_end) =
        calculate_cycle_boundaries(NaiveDate::from_ymd_opt(2026, 4, 15).unwrap());

    // Record attempt
    let mut tx1 = pool.begin().await.expect("Failed to begin transaction");
    let attempt_id = record_cycle_attempt(
        &mut tx1,
        tenant_id,
        subscription_id,
        cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await
    .expect("Failed to record attempt");
    tx1.commit().await.expect("Failed to commit transaction");

    // Mark as failed
    let mut tx2 = pool.begin().await.expect("Failed to begin transaction");
    mark_attempt_failed(&mut tx2, attempt_id, "AR_API_ERROR", "Failed to create invoice")
        .await
        .expect("Failed to mark failed");
    tx2.commit().await.expect("Failed to commit transaction");

    // Verify status
    let (status, failure_code, failure_message): (String, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT status::text, failure_code, failure_message
             FROM subscription_invoice_attempts WHERE id = $1",
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query attempt");

    assert_eq!(status, "failed_final");
    assert_eq!(failure_code, Some("AR_API_ERROR".to_string()));
    assert_eq!(failure_message, Some("Failed to create invoice".to_string()));

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Advisory Lock Tests
// ============================================================================

#[tokio::test]
async fn test_advisory_lock_basic() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;
    let cycle_key = "2026-05";

    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    // Acquire lock
    acquire_cycle_lock(&mut tx, tenant_id, subscription_id, cycle_key)
        .await
        .expect("Failed to acquire lock");

    // Lock is automatically released on commit
    tx.commit().await.expect("Failed to commit transaction");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_different_cycles_dont_block() {
    let pool = setup_test_pool().await;
    let tenant_id = &format!("tenant-{}", Uuid::new_v4());
    let subscription_id = create_test_subscription(&pool, tenant_id).await;

    // Two different cycles should not block each other
    let cycle_key1 = "2026-06";
    let cycle_key2 = "2026-07";

    let mut tx1 = pool.begin().await.expect("Failed to begin transaction");
    let mut tx2 = pool.begin().await.expect("Failed to begin transaction");

    // Acquire lock for cycle1
    acquire_cycle_lock(&mut tx1, tenant_id, subscription_id, cycle_key1)
        .await
        .expect("Failed to acquire lock for cycle1");

    // Acquire lock for cycle2 (should not block)
    acquire_cycle_lock(&mut tx2, tenant_id, subscription_id, cycle_key2)
        .await
        .expect("Failed to acquire lock for cycle2");

    tx1.commit().await.expect("Failed to commit tx1");
    tx2.commit().await.expect("Failed to commit tx2");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

//! Integration Tests for AR Finalization Gating (Phase 15 - bd-3fo)
//!
//! Tests verify:
//! 1. New attempt creation succeeds
//! 2. Duplicate attempts → deterministic no-op (AlreadyProcessed)
//! 3. Concurrency safety (SELECT FOR UPDATE prevents double-finalization)
//! 4. UNIQUE constraint enforcement
//! 5. Exactly-once side effects

use ar_rs::finalization::{finalize_invoice, FinalizationResult};
use ar_rs::lifecycle::status;
use chrono::Utc;
use dotenvy;
use futures::future::join_all;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Test helper: Ensure test customer exists
async fn ensure_test_customer(pool: &PgPool) {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM ar_customers WHERE app_id = 'app-test' AND email = 'test@example.com')"
    )
    .fetch_one(pool)
    .await
    .expect("Failed to check if customer exists");

    if !exists {
        sqlx::query(
            "INSERT INTO ar_customers (app_id, email, name, created_at, updated_at)
             VALUES ('app-test', 'test@example.com', 'Test Customer', $1, $2)",
        )
        .bind(Utc::now().naive_utc())
        .bind(Utc::now().naive_utc())
        .execute(pool)
        .await
        .expect("Failed to create test customer");
    }
}

/// Test helper: Get test customer ID
async fn get_test_customer_id(pool: &PgPool) -> i32 {
    sqlx::query_scalar(
        "SELECT id FROM ar_customers WHERE app_id = 'app-test' AND email = 'test@example.com'",
    )
    .fetch_one(pool)
    .await
    .expect("Failed to get test customer ID")
}

/// Test helper: Create a test invoice in OPEN status
async fn create_test_invoice(pool: &PgPool) -> i32 {
    ensure_test_customer(pool).await;
    let customer_id = get_test_customer_id(pool).await;

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices (app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency, created_at, updated_at)
         VALUES ($1, $2, $3, $4::text, $5, 'usd', $6, $7)
         RETURNING id"
    )
    .bind("app-test")
    .bind(format!("tilled-inv-{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(status::OPEN)
    .bind(1000)
    .bind(Utc::now().naive_utc())
    .bind(Utc::now().naive_utc())
    .fetch_one(pool)
    .await
    .expect("Failed to create test invoice");

    invoice_id
}

/// Test helper: Get invoice status from database
async fn get_invoice_status(pool: &PgPool, invoice_id: i32) -> String {
    sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(pool)
        .await
        .expect("Failed to get invoice status")
}

/// Test helper: Get attempt count for invoice
async fn get_attempt_count(pool: &PgPool, invoice_id: i32) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_attempts WHERE invoice_id = $1")
        .bind(invoice_id)
        .fetch_one(pool)
        .await
        .expect("Failed to get attempt count")
}

/// Test helper: Cleanup test data
async fn cleanup_test_data(pool: &PgPool) {
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = 'app-test'")
        .execute(pool)
        .await
        .expect("Failed to cleanup attempts");

    sqlx::query("DELETE FROM ar_invoices WHERE app_id = 'app-test'")
        .execute(pool)
        .await
        .expect("Failed to cleanup invoices");
}

/// Get database pool from environment
fn get_pool() -> PgPool {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL_AR")
        .expect("DATABASE_URL_AR must be set for integration tests");

    sqlx::PgPool::connect_lazy(&database_url).expect("Failed to create database pool")
}

// ============================================================================
// Basic Finalization Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_finalize_invoice_new_attempt() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool).await;

    // Finalize invoice (attempt 0)
    let result = finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("Finalization should succeed");

    // Verify result is NewAttempt
    assert!(
        matches!(result, FinalizationResult::NewAttempt { .. }),
        "First finalization should create new attempt"
    );

    if let FinalizationResult::NewAttempt {
        attempt_id,
        idempotency_key,
    } = result
    {
        assert!(!attempt_id.is_nil(), "Attempt ID should be valid UUID");
        assert!(
            idempotency_key.starts_with("invoice:attempt:app-test"),
            "Idempotency key should match format"
        );
    }

    // Verify invoice status transitioned to ATTEMPTING
    let status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(
        status,
        status::ATTEMPTING,
        "Invoice should transition to ATTEMPTING"
    );

    // Verify attempt row was created
    let attempt_count = get_attempt_count(&pool, invoice_id).await;
    assert_eq!(attempt_count, 1, "Should have exactly one attempt row");

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_finalize_invoice_duplicate_attempt() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool).await;

    // First finalization (attempt 0)
    let first_result = finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("First finalization should succeed");

    assert!(
        matches!(first_result, FinalizationResult::NewAttempt { .. }),
        "First finalization should create new attempt"
    );

    let first_attempt_id = match first_result {
        FinalizationResult::NewAttempt { attempt_id, .. } => attempt_id,
        _ => panic!("Expected NewAttempt"),
    };

    // Second finalization (duplicate attempt 0) - should be deterministic no-op
    let second_result = finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("Duplicate finalization should not error");

    // Verify result is AlreadyProcessed
    assert!(
        matches!(second_result, FinalizationResult::AlreadyProcessed { .. }),
        "Duplicate attempt should return AlreadyProcessed"
    );

    if let FinalizationResult::AlreadyProcessed {
        existing_attempt_id,
        ..
    } = second_result
    {
        assert_eq!(
            existing_attempt_id, first_attempt_id,
            "Should return same attempt ID as first finalization"
        );
    }

    // Verify only ONE attempt row exists (no duplicate)
    let attempt_count = get_attempt_count(&pool, invoice_id).await;
    assert_eq!(
        attempt_count, 1,
        "Should still have exactly one attempt row (no duplicate)"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_finalize_invoice_multiple_attempts() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool).await;

    // Finalize with attempt 0
    let result0 = finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("Attempt 0 should succeed");
    assert!(matches!(result0, FinalizationResult::NewAttempt { .. }));

    // Finalize with attempt 1 (different attempt number)
    let result1 = finalize_invoice(&pool, "app-test", invoice_id, 1)
        .await
        .expect("Attempt 1 should succeed");
    assert!(matches!(result1, FinalizationResult::NewAttempt { .. }));

    // Finalize with attempt 2
    let result2 = finalize_invoice(&pool, "app-test", invoice_id, 2)
        .await
        .expect("Attempt 2 should succeed");
    assert!(matches!(result2, FinalizationResult::NewAttempt { .. }));

    // Verify 3 distinct attempt rows exist
    let attempt_count = get_attempt_count(&pool, invoice_id).await;
    assert_eq!(
        attempt_count, 3,
        "Should have 3 attempt rows (one per attempt number)"
    );

    // Verify duplicate of attempt 1 is rejected
    let duplicate_result = finalize_invoice(&pool, "app-test", invoice_id, 1)
        .await
        .expect("Duplicate attempt 1 should not error");
    assert!(matches!(
        duplicate_result,
        FinalizationResult::AlreadyProcessed { .. }
    ));

    // Verify still only 3 attempt rows (no new row from duplicate)
    let attempt_count_after = get_attempt_count(&pool, invoice_id).await;
    assert_eq!(
        attempt_count_after, 3,
        "Should still have 3 attempt rows after duplicate"
    );

    cleanup_test_data(&pool).await;
}

// ============================================================================
// Concurrency Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_finalize_invoice_concurrent_attempts() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool).await;

    // Spawn 10 concurrent finalization attempts (all attempt 0)
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let pool_clone = pool.clone();
        let task =
            tokio::spawn(
                async move { finalize_invoice(&pool_clone, "app-test", invoice_id, 0).await },
            );
        tasks.push(task);
    }

    // Wait for all tasks to complete
    let results = join_all(tasks).await;

    // Count successes (NewAttempt) and no-ops (AlreadyProcessed)
    let mut new_attempts = 0;
    let mut already_processed = 0;

    for result in results {
        match result.expect("Task should not panic") {
            Ok(FinalizationResult::NewAttempt { .. }) => new_attempts += 1,
            Ok(FinalizationResult::AlreadyProcessed { .. }) => already_processed += 1,
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    // Exactly ONE should succeed, rest should be no-ops
    assert_eq!(
        new_attempts, 1,
        "Exactly one concurrent finalization should succeed"
    );
    assert_eq!(
        already_processed, 9,
        "Nine concurrent finalizations should be no-ops"
    );

    // Verify exactly ONE attempt row was created
    let attempt_count = get_attempt_count(&pool, invoice_id).await;
    assert_eq!(
        attempt_count, 1,
        "Exactly one attempt row should exist after concurrent finalizations"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_finalize_invoice_concurrent_different_attempts() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool).await;

    // Spawn concurrent finalization attempts with different attempt numbers
    let mut tasks = Vec::new();

    // 5 tasks for attempt 0
    for _ in 0..5 {
        let pool_clone = pool.clone();
        let task =
            tokio::spawn(
                async move { finalize_invoice(&pool_clone, "app-test", invoice_id, 0).await },
            );
        tasks.push(task);
    }

    // 5 tasks for attempt 1
    for _ in 0..5 {
        let pool_clone = pool.clone();
        let task =
            tokio::spawn(
                async move { finalize_invoice(&pool_clone, "app-test", invoice_id, 1).await },
            );
        tasks.push(task);
    }

    // 5 tasks for attempt 2
    for _ in 0..5 {
        let pool_clone = pool.clone();
        let task =
            tokio::spawn(
                async move { finalize_invoice(&pool_clone, "app-test", invoice_id, 2).await },
            );
        tasks.push(task);
    }

    // Wait for all tasks to complete
    let results = join_all(tasks).await;

    // Count new attempts (should be 3: one for each attempt number)
    let mut new_attempts = 0;
    let mut already_processed = 0;

    for result in results {
        match result.expect("Task should not panic") {
            Ok(FinalizationResult::NewAttempt { .. }) => new_attempts += 1,
            Ok(FinalizationResult::AlreadyProcessed { .. }) => already_processed += 1,
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    // Exactly 3 should succeed (one per attempt number), rest should be no-ops
    assert_eq!(
        new_attempts, 3,
        "Exactly three concurrent finalizations should succeed (one per attempt number)"
    );
    assert_eq!(
        already_processed, 12,
        "Twelve concurrent finalizations should be no-ops"
    );

    // Verify exactly 3 attempt rows exist (one per attempt number)
    let attempt_count = get_attempt_count(&pool, invoice_id).await;
    assert_eq!(
        attempt_count, 3,
        "Exactly 3 attempt rows should exist (one per attempt number)"
    );

    cleanup_test_data(&pool).await;
}

// ============================================================================
// Error Cases
// ============================================================================

#[tokio::test]
#[serial]
async fn test_finalize_invoice_not_found() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Attempt to finalize non-existent invoice
    let result = finalize_invoice(&pool, "app-test", 999999, 0).await;

    assert!(
        result.is_err(),
        "Finalization should fail for non-existent invoice"
    );

    if let Err(e) = result {
        assert!(
            e.to_string().contains("Invoice not found"),
            "Error should indicate invoice not found"
        );
    }

    cleanup_test_data(&pool).await;
}

// ============================================================================
// Idempotency Key Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_finalize_invoice_idempotency_key_format() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool).await;

    // Finalize invoice
    let result = finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("Finalization should succeed");

    if let FinalizationResult::NewAttempt {
        idempotency_key, ..
    } = result
    {
        // Verify idempotency key format
        let expected = format!("invoice:attempt:app-test:{}:0", invoice_id);
        assert_eq!(
            idempotency_key, expected,
            "Idempotency key should match deterministic format"
        );

        // Verify key is stored in database
        let stored_key: String = sqlx::query_scalar(
            "SELECT idempotency_key FROM ar_invoice_attempts WHERE invoice_id = $1 AND attempt_no = $2"
        )
        .bind(invoice_id)
        .bind(0)
        .fetch_one(&pool)
        .await
        .expect("Should find attempt row");

        assert_eq!(stored_key, expected, "Stored idempotency key should match");
    } else {
        panic!("Expected NewAttempt result");
    }

    cleanup_test_data(&pool).await;
}

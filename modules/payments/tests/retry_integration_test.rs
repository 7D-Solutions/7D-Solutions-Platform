//! Integration tests for Payments Retry Window Discipline (bd-1it)
//!
//! Tests verify:
//! 1. Retry window calculation and eligibility (unit-level)
//! 2. UNKNOWN blocking protocol (bd-2uw integration)
//! 3. Exactly-once enforcement via UNIQUE constraints
//! 4. Retry scheduling using attempted_at anchor (no AR cross-module dependency)
//!
//! NOTE: get_payments_for_retry uses the first attempt's attempted_at as the
//! retry anchor (bd-2wtz module isolation). AR invoice due_at is NOT used.

use sqlx::PgPool;
use uuid::Uuid;

// Test helper to create a test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5434/payments_test".to_string()
    });

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// Test helper to create a payment attempt with a specific attempted_at timestamp
async fn create_payment_attempt_at(
    pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: &str,
    attempt_no: i32,
    status: &str,
    attempted_at: &str,
) -> Uuid {
    let attempt_id: Uuid = sqlx::query_scalar(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status, attempted_at)
         VALUES ($1, $2, $3, $4, $5::payment_attempt_status, $6::timestamp)
         RETURNING id"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id)
    .bind(attempt_no)
    .bind(status)
    .bind(attempted_at)
    .fetch_one(pool)
    .await
    .expect("Failed to create payment attempt");

    attempt_id
}

// Test helper to create a payment attempt with NOW() as attempted_at
async fn create_payment_attempt(
    pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: &str,
    attempt_no: i32,
    status: &str,
) -> Uuid {
    let attempt_id: Uuid = sqlx::query_scalar(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3, $4, $5::payment_attempt_status)
         RETURNING id"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id)
    .bind(attempt_no)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to create payment attempt");

    attempt_id
}

// Clean up payment attempts for a payment_id
async fn cleanup_payment(pool: &PgPool, payment_id: Uuid) {
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Retry Window Calculation Tests (unit-level, no DB required)
// ============================================================================

#[tokio::test]
async fn test_retry_windows_calculation() {
    use chrono::NaiveDate;
    use payments_rs::retry::{calculate_retry_windows, determine_current_window};

    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let windows = calculate_retry_windows(due_date);

    assert_eq!(windows[0], NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());
    assert_eq!(windows[1], NaiveDate::from_ymd_opt(2026, 2, 18).unwrap());
    assert_eq!(windows[2], NaiveDate::from_ymd_opt(2026, 2, 22).unwrap());

    // Test window determination
    let today = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(0));

    let today = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(1));

    let today = NaiveDate::from_ymd_opt(2026, 2, 22).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(2));
}

// ============================================================================
// Eligibility Tests (unit-level, no DB required)
// ============================================================================

#[tokio::test]
async fn test_eligibility_checks() {
    use payments_rs::retry::is_eligible_for_retry;

    // Eligible statuses
    assert!(is_eligible_for_retry("attempting"));
    assert!(is_eligible_for_retry("failed_retry"));

    // Ineligible statuses
    assert!(!is_eligible_for_retry("succeeded"));
    assert!(!is_eligible_for_retry("failed_final"));

    // CRITICAL: UNKNOWN blocks retry
    assert!(!is_eligible_for_retry("unknown"));
}

// ============================================================================
// UNKNOWN Blocking Protocol Tests (bd-2uw Integration)
// ============================================================================

#[tokio::test]
async fn test_unknown_status_blocks_retry() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create payment attempt with status='unknown', anchored in the past
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 0, "unknown",
        "2026-01-01 00:00:00",
    ).await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // CRITICAL ASSERTION: UNKNOWN status must be excluded from retry list
    assert!(
        retry_list.is_empty(),
        "Payment with status='unknown' must not appear in retry list (UNKNOWN blocking protocol)"
    );

    cleanup_payment(&pool, payment_id).await;
}

#[tokio::test]
async fn test_failed_retry_is_eligible() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create payment attempt with status='failed_retry', anchored far in the past
    // so that multiple retry windows are now active
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 0, "failed_retry",
        "2026-01-01 00:00:00",
    ).await;

    // Query for retry-eligible payments (today is well past anchor date)
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: failed_retry status should be eligible for retry
    assert!(
        !retry_list.is_empty(),
        "Payment with status='failed_retry' should be eligible for retry"
    );

    cleanup_payment(&pool, payment_id).await;
}

// ============================================================================
// Exactly-Once Enforcement Tests
// ============================================================================

#[tokio::test]
async fn test_no_duplicate_attempts_per_window() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create attempt 0 in a past window with 'attempting' status
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 0, "attempting",
        "2026-01-01 00:00:00",
    ).await;

    // Also create attempt 1 to fill the first retry window
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 1, "attempting",
        "2026-01-04 00:00:00",
    ).await;

    // Also create attempt 2 to fill the second retry window
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 2, "attempting",
        "2026-01-08 00:00:00",
    ).await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: Should not return payment because attempts exist for all windows
    let has_payment = retry_list.iter().any(|(id, _)| *id == payment_id);
    assert!(
        !has_payment,
        "Payment should not be in retry list if attempts already exist for all windows"
    );

    cleanup_payment(&pool, payment_id).await;
}

#[tokio::test]
async fn test_unique_constraint_prevents_duplicates() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create first attempt
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "attempting").await;

    // Try to create duplicate attempt (same app_id, payment_id, attempt_no)
    let result = sqlx::query(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3, $4, $5::payment_attempt_status)"
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(&invoice_id)
    .bind(0)
    .bind("attempting")
    .execute(&pool)
    .await;

    // ASSERTION: Duplicate insert should fail with UNIQUE constraint violation
    assert!(
        result.is_err(),
        "UNIQUE constraint should prevent duplicate (app_id, payment_id, attempt_no)"
    );

    cleanup_payment(&pool, payment_id).await;
}

// ============================================================================
// Retry Scheduling Uses attempted_at Anchor (bd-2wtz Module Isolation)
// ============================================================================

#[tokio::test]
async fn test_retry_scheduling_uses_attempted_at_anchor() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create attempt 0 with a timestamp far enough in the past that window 1 (+3 days) is now active
    // Use a date 10 days ago so all windows have passed
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 0, "failed_retry",
        "2026-01-01 00:00:00",
    ).await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Retry scheduling using attempted_at anchor must succeed");

    // ASSERTION: Should return payment (windows 1 or 2 are active based on attempted_at)
    let found = retry_list.iter().find(|(id, _)| *id == payment_id);
    assert!(
        found.is_some(),
        "Payment should be eligible for retry based on attempted_at anchor (no AR dependency)"
    );

    if let Some((_, attempt_no)) = found {
        assert!(
            *attempt_no > 0,
            "Next attempt should be in window 1 or 2 (not 0, which already exists)"
        );
    }

    cleanup_payment(&pool, payment_id).await;
}

#[tokio::test]
async fn test_payment_with_no_attempt_zero_excluded() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create only attempt 1 (no attempt_no=0 exists) — scheduler requires attempt 0 as anchor
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 1, "failed_retry",
        "2026-01-01 00:00:00",
    ).await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: Payment with no attempt_no=0 is excluded (no anchor date)
    let has_payment = retry_list.iter().any(|(id, _)| *id == payment_id);
    assert!(
        !has_payment,
        "Payment with no attempt_no=0 should be excluded (no retry anchor date)"
    );

    cleanup_payment(&pool, payment_id).await;
}

// ============================================================================
// Multi-Window Tests
// ============================================================================

#[tokio::test]
async fn test_multi_window_progression() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // Create attempt 0 far in the past (all windows active)
    create_payment_attempt_at(
        &pool, app_id, payment_id, &invoice_id, 0, "failed_retry",
        "2026-01-01 00:00:00",
    ).await;

    // Query for retry-eligible payments (today >> anchor + 7 days)
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: Should return payment for next window (attempt_no > 0)
    let found = retry_list.iter().find(|(id, _)| *id == payment_id);
    assert!(
        found.is_some(),
        "Payment should be eligible for next retry window"
    );

    if let Some((_, attempt_no)) = found {
        assert!(
            *attempt_no > 0,
            "Next attempt should be in window 1 or 2 (not 0)"
        );
    }

    cleanup_payment(&pool, payment_id).await;
}

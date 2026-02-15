//! Integration tests for Payments Retry Window Discipline (bd-1it)
//!
//! Tests verify:
//! 1. Retry window calculation and eligibility
//! 2. UNKNOWN blocking protocol (bd-2uw integration)
//! 3. Exactly-once enforcement via UNIQUE constraints
//! 4. Cross-module integration with AR invoices (due_at dates)

use chrono::NaiveDate;
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

// Test helper to create an AR invoice with due date
async fn create_ar_invoice_with_due_date(
    pool: &PgPool,
    app_id: &str,
    due_date: NaiveDate,
) -> String {
    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar.ar_invoices (app_id, ar_customer_id, status, amount_cents, currency, due_at)
         VALUES ($1, 'cust-123', 'open', 10000, 'USD', $2)
         RETURNING id"
    )
    .bind(app_id)
    .bind(due_date)
    .fetch_one(pool)
    .await
    .expect("Failed to create AR invoice");

    invoice_id.to_string()
}

// Test helper to create a payment attempt
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

// ============================================================================
// Retry Window Calculation Tests
// ============================================================================

#[tokio::test]
async fn test_retry_windows_calculation() {
    // Unit test coverage via module tests
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
// Eligibility Tests
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

    // Create AR invoice with due date in the past (eligible window)
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let invoice_id = create_ar_invoice_with_due_date(&pool, app_id, due_date).await;

    // Create payment attempt with status='unknown'
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "unknown").await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // CRITICAL ASSERTION: UNKNOWN status must be excluded from retry list
    assert!(
        retry_list.is_empty(),
        "Payment with status='unknown' must not appear in retry list (UNKNOWN blocking protocol)"
    );

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_failed_retry_is_eligible() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();

    // Create AR invoice with due date in the past (attempt 1 window: +3 days)
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let invoice_id = create_ar_invoice_with_due_date(&pool, app_id, due_date).await;

    // Create payment attempt with status='failed_retry' and attempt_no=0
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "failed_retry").await;

    // Query for retry-eligible payments (today is well past due date)
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: failed_retry status should be eligible for retry
    assert!(
        !retry_list.is_empty(),
        "Payment with status='failed_retry' should be eligible for retry"
    );

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Exactly-Once Enforcement Tests
// ============================================================================

#[tokio::test]
async fn test_no_duplicate_attempts_per_window() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();

    // Create AR invoice with due date in the past
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let invoice_id = create_ar_invoice_with_due_date(&pool, app_id, due_date).await;

    // Create attempt for window 0
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "attempting").await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: Should not return payment because attempt already exists for window 0
    let has_payment = retry_list.iter().any(|(id, _)| *id == payment_id);
    assert!(
        !has_payment,
        "Payment should not be in retry list if attempt already exists for current window"
    );

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_unique_constraint_prevents_duplicates() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();

    // Create AR invoice
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let invoice_id = create_ar_invoice_with_due_date(&pool, app_id, due_date).await;

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

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Cross-Module Integration Tests (AR Invoices)
// ============================================================================

#[tokio::test]
async fn test_join_to_ar_invoices_for_due_date() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();

    // Create AR invoice with specific due date
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let invoice_id = create_ar_invoice_with_due_date(&pool, app_id, due_date).await;

    // Create payment attempt referencing the invoice
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "attempting").await;

    // Verify the JOIN works by querying for retry-eligible payments
    // (This implicitly tests the JOIN to ar.ar_invoices)
    let result = payments_rs::retry::get_payments_for_retry(&pool, app_id).await;
    assert!(result.is_ok(), "JOIN to ar.ar_invoices should succeed");

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_payment_without_due_date_excluded() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();

    // Create AR invoice WITHOUT due date
    let invoice_id: String = sqlx::query_scalar(
        "INSERT INTO ar.ar_invoices (app_id, ar_customer_id, status, amount_cents, currency)
         VALUES ($1, 'cust-123', 'open', 10000, 'USD')
         RETURNING id::text"
    )
    .bind(app_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to create AR invoice");

    // Create payment attempt with status='attempting'
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "attempting").await;

    // Query for retry-eligible payments
    let retry_list = payments_rs::retry::get_payments_for_retry(&pool, app_id)
        .await
        .expect("Failed to get payments for retry");

    // ASSERTION: Payment without due date should be excluded from retry list
    assert!(
        retry_list.is_empty(),
        "Payment without AR invoice due_at should be excluded from retry list"
    );

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Multi-Window Tests
// ============================================================================

#[tokio::test]
async fn test_multi_window_progression() {
    let pool = setup_test_pool().await;
    let app_id = &format!("app-{}", Uuid::new_v4());
    let payment_id = Uuid::new_v4();

    // Create AR invoice with due date far in the past (all windows active)
    let due_date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let invoice_id = create_ar_invoice_with_due_date(&pool, app_id, due_date).await;

    // Create attempt for window 0 only
    create_payment_attempt(&pool, app_id, payment_id, &invoice_id, 0, "failed_retry").await;

    // Query for retry-eligible payments (today >> due date + 7 days)
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

    // Clean up
    sqlx::query("DELETE FROM payment_attempts WHERE payment_id = $1")
        .bind(payment_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar.ar_invoices WHERE id = $1::integer")
        .bind(&invoice_id)
        .execute(&pool)
        .await
        .ok();
}

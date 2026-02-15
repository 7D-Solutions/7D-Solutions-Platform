//! Integration Tests for AR Retry Window Discipline (Phase 15 - bd-8ev)
//!
//! Tests verify:
//! 1. Retry window calculation correctness
//! 2. Window determination based on current date
//! 3. Exactly one attempt per window enforcement
//! 4. get_invoices_for_retry accuracy
//! 5. Integration with finalization gating

use ar_rs::finalization::finalize_invoice;
use ar_rs::lifecycle::status;
use ar_rs::retry::{calculate_retry_windows, determine_current_window, get_invoices_for_retry, is_eligible_for_retry, windows};
use chrono::{NaiveDate, Utc};
use dotenvy;
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
             VALUES ('app-test', 'test@example.com', 'Test Customer', $1, $2)"
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
        "SELECT id FROM ar_customers WHERE app_id = 'app-test' AND email = 'test@example.com'"
    )
    .fetch_one(pool)
    .await
    .expect("Failed to get test customer ID")
}

/// Test helper: Create a test invoice with specific due date
async fn create_test_invoice_with_due_date(pool: &PgPool, due_date: NaiveDate) -> i32 {
    ensure_test_customer(pool).await;
    let customer_id = get_test_customer_id(pool).await;

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices (app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4::text, $5, 'usd', $6, $7, $8)
         RETURNING id"
    )
    .bind("app-test")
    .bind(format!("tilled-inv-{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(status::OPEN)
    .bind(1000)
    .bind(due_date.and_hms_opt(0, 0, 0).unwrap()) // Convert to timestamp
    .bind(Utc::now().naive_utc())
    .bind(Utc::now().naive_utc())
    .fetch_one(pool)
    .await
    .expect("Failed to create test invoice");

    invoice_id
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

    sqlx::PgPool::connect_lazy(&database_url)
        .expect("Failed to create database pool")
}

// ============================================================================
// Window Calculation Tests
// ============================================================================

#[test]
fn test_calculate_retry_windows() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let windows = calculate_retry_windows(due_date);

    // Attempt 0: immediate (due date)
    assert_eq!(windows[0], NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());

    // Attempt 1: +3 days
    assert_eq!(windows[1], NaiveDate::from_ymd_opt(2026, 2, 18).unwrap());

    // Attempt 2: +7 days
    assert_eq!(windows[2], NaiveDate::from_ymd_opt(2026, 2, 22).unwrap());
}

#[test]
fn test_window_offsets() {
    assert_eq!(windows::ATTEMPT_0_OFFSET_DAYS, 0);
    assert_eq!(windows::ATTEMPT_1_OFFSET_DAYS, 3);
    assert_eq!(windows::ATTEMPT_2_OFFSET_DAYS, 7);
    assert_eq!(windows::MAX_ATTEMPTS, 3);
}

// ============================================================================
// Window Determination Tests
// ============================================================================

#[test]
fn test_determine_current_window_before_due() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 2, 14).unwrap();

    assert_eq!(
        determine_current_window(due_date, today),
        None,
        "No window active before due date"
    );
}

#[test]
fn test_determine_current_window_on_due_date() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    assert_eq!(
        determine_current_window(due_date, today),
        Some(0),
        "Attempt 0 window active on due date"
    );
}

#[test]
fn test_determine_current_window_between_0_and_1() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // Day 1 after due date (still in attempt 0 window)
    let today = NaiveDate::from_ymd_opt(2026, 2, 16).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(0));

    // Day 2 after due date (still in attempt 0 window)
    let today = NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(0));
}

#[test]
fn test_determine_current_window_on_retry_1() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // On +3 days
    let today = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
    assert_eq!(
        determine_current_window(due_date, today),
        Some(1),
        "Attempt 1 window active on +3 days"
    );
}

#[test]
fn test_determine_current_window_between_1_and_2() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // Days between +3 and +7
    let today = NaiveDate::from_ymd_opt(2026, 2, 19).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(1));

    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(1));

    let today = NaiveDate::from_ymd_opt(2026, 2, 21).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(1));
}

#[test]
fn test_determine_current_window_on_retry_2() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // On +7 days
    let today = NaiveDate::from_ymd_opt(2026, 2, 22).unwrap();
    assert_eq!(
        determine_current_window(due_date, today),
        Some(2),
        "Attempt 2 window active on +7 days"
    );
}

#[test]
fn test_determine_current_window_after_all_windows() {
    let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

    // After +7 days (still in attempt 2 window - last window stays open)
    let today = NaiveDate::from_ymd_opt(2026, 2, 23).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(2));

    let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    assert_eq!(determine_current_window(due_date, today), Some(2));
}

// ============================================================================
// Eligibility Tests
// ============================================================================

#[test]
fn test_is_eligible_for_retry() {
    // Eligible statuses
    assert!(is_eligible_for_retry("open"));
    assert!(is_eligible_for_retry("attempting"));
    assert!(is_eligible_for_retry("failed_retry"));

    // Ineligible statuses (terminal)
    assert!(!is_eligible_for_retry("paid"));
    assert!(!is_eligible_for_retry("failed_final"));
    assert!(!is_eligible_for_retry("void"));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_invoices_for_retry_empty() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    let invoices = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");

    assert_eq!(
        invoices.len(),
        0,
        "Should return empty list when no invoices exist"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_invoices_for_retry_single_invoice() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice due today
    let due_date = Utc::now().date_naive();
    let invoice_id = create_test_invoice_with_due_date(&pool, due_date).await;

    // Get invoices for retry
    let invoices = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");

    assert_eq!(invoices.len(), 1, "Should return one invoice");
    assert_eq!(invoices[0].0, invoice_id, "Should return correct invoice ID");
    assert_eq!(invoices[0].1, 0, "Should be in attempt 0 window");

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_invoices_for_retry_excludes_existing_attempt() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice due today
    let due_date = Utc::now().date_naive();
    let invoice_id = create_test_invoice_with_due_date(&pool, due_date).await;

    // Create attempt 0 via finalization
    finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("First finalization should succeed");

    // Get invoices for retry
    let invoices = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");

    assert_eq!(
        invoices.len(),
        0,
        "Should exclude invoice with existing attempt for current window"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_invoices_for_retry_multiple_windows() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    let today = Utc::now().date_naive();

    // Create invoice 1: due today (window 0)
    let invoice_1 = create_test_invoice_with_due_date(&pool, today).await;

    // Create invoice 2: due 3 days ago (window 1)
    let invoice_2 = create_test_invoice_with_due_date(
        &pool,
        today - chrono::Days::new(3),
    )
    .await;

    // Create invoice 3: due 7 days ago (window 2)
    let invoice_3 = create_test_invoice_with_due_date(
        &pool,
        today - chrono::Days::new(7),
    )
    .await;

    // Get invoices for retry
    let invoices = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");

    assert_eq!(invoices.len(), 3, "Should return all 3 invoices");

    // Find each invoice in results
    let inv1_result = invoices.iter().find(|(id, _)| *id == invoice_1);
    let inv2_result = invoices.iter().find(|(id, _)| *id == invoice_2);
    let inv3_result = invoices.iter().find(|(id, _)| *id == invoice_3);

    assert!(inv1_result.is_some(), "Invoice 1 should be in results");
    assert!(inv2_result.is_some(), "Invoice 2 should be in results");
    assert!(inv3_result.is_some(), "Invoice 3 should be in results");

    assert_eq!(inv1_result.unwrap().1, 0, "Invoice 1 should be in window 0");
    assert_eq!(inv2_result.unwrap().1, 1, "Invoice 2 should be in window 1");
    assert_eq!(inv3_result.unwrap().1, 2, "Invoice 3 should be in window 2");

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_invoices_for_retry_excludes_terminal_status() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    let due_date = Utc::now().date_naive();

    // Create invoice and set to PAID (terminal)
    let invoice_id = create_test_invoice_with_due_date(&pool, due_date).await;

    sqlx::query("UPDATE ar_invoices SET status = $1 WHERE id = $2")
        .bind(status::PAID)
        .bind(invoice_id)
        .execute(&pool)
        .await
        .expect("Failed to update status");

    // Get invoices for retry
    let invoices = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");

    assert_eq!(
        invoices.len(),
        0,
        "Should exclude invoice with terminal status (PAID)"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_retry_window_exactly_once_enforcement() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    let due_date = Utc::now().date_naive();
    let invoice_id = create_test_invoice_with_due_date(&pool, due_date).await;

    // First check: invoice appears for retry
    let invoices_first = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");
    assert_eq!(invoices_first.len(), 1, "Invoice should appear for retry");
    assert_eq!(invoices_first[0].1, 0, "Should be attempt 0");

    // Finalize attempt 0
    finalize_invoice(&pool, "app-test", invoice_id, 0)
        .await
        .expect("Finalization should succeed");

    // Second check: invoice should NOT appear again (exactly-once per window)
    let invoices_second = get_invoices_for_retry(&pool, "app-test")
        .await
        .expect("Should succeed");
    assert_eq!(
        invoices_second.len(),
        0,
        "Invoice should not appear again after attempt created (exactly-once per window)"
    );

    cleanup_test_data(&pool).await;
}

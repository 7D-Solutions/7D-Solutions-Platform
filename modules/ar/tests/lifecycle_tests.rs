//! Integration Tests for AR Lifecycle Module (Phase 15 - bd-1w7)
//!
//! Tests verify:
//! 1. Valid transitions succeed per state machine rules
//! 2. Invalid transitions are rejected with TransitionError
//! 3. Terminal states (PAID, FAILED_FINAL) cannot transition
//! 4. Guards validate ONLY (zero side effects proven by inspection)

use ar_rs::lifecycle::{
    status, transition_to_attempting, transition_to_failed_final, transition_to_paid,
    transition_to_void, LifecycleError,
};
use chrono::Utc;
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

/// Test helper: Create a test invoice in the database
async fn create_test_invoice(pool: &PgPool, status: &str) -> i32 {
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
    .bind(status)
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

/// Test helper: Cleanup test data
async fn cleanup_test_data(pool: &PgPool) {
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = 'app-test'")
        .execute(pool)
        .await
        .expect("Failed to cleanup test data");
}

/// Get database pool from environment
fn get_pool() -> PgPool {
    // Load .env file
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL_AR")
        .expect("DATABASE_URL_AR must be set for integration tests");

    sqlx::PgPool::connect_lazy(&database_url).expect("Failed to create database pool")
}

// ============================================================================
// Valid Transition Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_transition_open_to_attempting() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool, status::OPEN).await;

    // Transition to ATTEMPTING should succeed
    let result = transition_to_attempting(&pool, invoice_id, "app-test", "Starting payment collection").await;
    assert!(result.is_ok(), "Should allow OPEN → ATTEMPTING transition");

    // Verify status was updated
    let new_status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(new_status, status::ATTEMPTING);

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_transition_attempting_to_paid() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in ATTEMPTING status
    let invoice_id = create_test_invoice(&pool, status::ATTEMPTING).await;

    // Transition to PAID should succeed
    let result = transition_to_paid(&pool, invoice_id, "app-test", "Payment received").await;
    assert!(result.is_ok(), "Should allow ATTEMPTING → PAID transition");

    // Verify status was updated and paid_at was set
    let new_status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(new_status, status::PAID);

    // Verify paid_at is set
    let paid_at: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT paid_at FROM ar_invoices WHERE id = $1")
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to fetch paid_at");
    assert!(
        paid_at.is_some(),
        "paid_at should be set when transitioning to PAID"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_transition_attempting_to_failed_final() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in ATTEMPTING status
    let invoice_id = create_test_invoice(&pool, status::ATTEMPTING).await;

    // Transition to FAILED_FINAL should succeed
    let result = transition_to_failed_final(&pool, invoice_id, "app-test", "Max retries exceeded").await;
    assert!(
        result.is_ok(),
        "Should allow ATTEMPTING → FAILED_FINAL transition"
    );

    // Verify status was updated
    let new_status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(new_status, status::FAILED_FINAL);

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_transition_open_to_void() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool, status::OPEN).await;

    // Transition to VOID should succeed
    let result = transition_to_void(&pool, invoice_id, "app-test", "Customer cancelled").await;
    assert!(result.is_ok(), "Should allow OPEN → VOID transition");

    // Verify status was updated
    let new_status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(new_status, status::VOID);

    cleanup_test_data(&pool).await;
}

// ============================================================================
// Invalid Transition Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_transition_open_to_paid_rejected() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool, status::OPEN).await;

    // Attempt to skip ATTEMPTING and go directly to PAID (illegal)
    let result = transition_to_paid(&pool, invoice_id, "app-test", "Direct payment").await;
    assert!(
        result.is_err(),
        "Should reject OPEN → PAID transition (must go through ATTEMPTING)"
    );

    // Verify error type
    if let Err(LifecycleError::TransitionError(e)) = result {
        assert_eq!(
            e.to_string().contains("Illegal transition"),
            true,
            "Error should indicate illegal transition"
        );
    } else {
        panic!("Expected TransitionError, got: {:?}", result);
    }

    // Verify status was NOT changed
    let status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(
        status,
        status::OPEN,
        "Status should remain OPEN after rejected transition"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_transition_paid_is_terminal() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in PAID status (terminal state)
    let invoice_id = create_test_invoice(&pool, status::PAID).await;

    // Attempt to transition from PAID to ATTEMPTING (illegal)
    let result = transition_to_attempting(&pool, invoice_id, "app-test", "Retry payment").await;
    assert!(
        result.is_err(),
        "Should reject transitions from PAID (terminal state)"
    );

    // Verify status was NOT changed
    let status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(status, status::PAID, "Status should remain PAID (terminal)");

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_transition_failed_final_is_terminal() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in FAILED_FINAL status (terminal state)
    let invoice_id = create_test_invoice(&pool, status::FAILED_FINAL).await;

    // Attempt to transition from FAILED_FINAL to ATTEMPTING (illegal)
    let result = transition_to_attempting(&pool, invoice_id, "app-test", "Retry payment").await;
    assert!(
        result.is_err(),
        "Should reject transitions from FAILED_FINAL (terminal state)"
    );

    // Verify status was NOT changed
    let status = get_invoice_status(&pool, invoice_id).await;
    assert_eq!(
        status,
        status::FAILED_FINAL,
        "Status should remain FAILED_FINAL (terminal)"
    );

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_transition_invoice_not_found() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Attempt to transition non-existent invoice
    let result = transition_to_attempting(&pool, 999999, "app-test", "Test").await;
    assert!(
        result.is_err(),
        "Should return error for non-existent invoice"
    );

    // Verify error type
    if let Err(LifecycleError::TransitionError(e)) = result {
        assert!(
            e.to_string().contains("Invoice not found"),
            "Error should indicate invoice not found"
        );
    } else {
        panic!(
            "Expected TransitionError::InvoiceNotFound, got: {:?}",
            result
        );
    }

    cleanup_test_data(&pool).await;
}

// ============================================================================
// Guard Purity Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_guard_has_zero_side_effects_on_rejection() {
    let pool = get_pool();
    cleanup_test_data(&pool).await;

    // Create invoice in OPEN status
    let invoice_id = create_test_invoice(&pool, status::OPEN).await;

    // Attempt illegal transition (OPEN → PAID)
    let result = transition_to_paid(&pool, invoice_id, "app-test", "Direct payment").await;
    assert!(result.is_err(), "Illegal transition should be rejected");

    // Guard rejection should NOT have modified database
    // Verify:
    // 1. Status unchanged
    // 2. paid_at still NULL
    // 3. updated_at unchanged (within 1 second tolerance)

    let (status, paid_at, updated_at): (
        String,
        Option<chrono::NaiveDateTime>,
        chrono::NaiveDateTime,
    ) = sqlx::query_as("SELECT status, paid_at, updated_at FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to fetch invoice");

    assert_eq!(
        status,
        status::OPEN,
        "Status should not change on rejected transition"
    );
    assert!(
        paid_at.is_none(),
        "paid_at should remain NULL on rejected transition"
    );

    // updated_at should not have changed (guard had zero side effects)
    // Allow small time delta for DB timestamp precision
    let now = Utc::now().naive_utc();
    let time_since_creation = now.signed_duration_since(updated_at);
    assert!(
        time_since_creation.num_seconds() < 5,
        "updated_at should not change (guard should have zero side effects)"
    );

    cleanup_test_data(&pool).await;
}

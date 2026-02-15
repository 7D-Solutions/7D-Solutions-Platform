// Phase 15: Invoice Attempt Ledger Tests
// Tests verify UNIQUE constraint enforcement and attempt tracking

mod common;

use common::setup_pool;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup attempts");
}

/// Helper to create a test invoice
async fn create_test_invoice(pool: &PgPool, app_id: &str) -> i32 {
    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, status) VALUES ($1, $2, 'active') RETURNING id",
    )
    .bind(app_id)
    .bind(format!("test-{}@example.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("Failed to create test customer");

    sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, updated_at
        )
        VALUES ($1, $2, $3, 'open', 1000, 'usd', CURRENT_TIMESTAMP)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test invoice")
}

#[tokio::test]
#[serial]
async fn test_invoice_attempt_unique_constraint() {
    let pool = setup_pool().await;
    let app_id = "test_app";

    // Cleanup any existing test data
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup attempts");

    // Create test invoice
    let invoice_id = create_test_invoice(&pool, app_id).await;

    // First attempt should succeed
    let result = sqlx::query(
        r#"
        INSERT INTO ar_invoice_attempts (
            app_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 'attempting')
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(0)
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "First attempt insertion should succeed");

    // Duplicate attempt should fail (UNIQUE constraint)
    let result = sqlx::query(
        r#"
        INSERT INTO ar_invoice_attempts (
            app_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 'attempting')
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(0) // Same attempt_no
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "Duplicate attempt insertion should fail due to UNIQUE constraint"
    );

    // Verify error is a unique constraint violation (23505)
    if let Err(sqlx::Error::Database(db_err)) = result {
        assert_eq!(
            db_err.code().as_deref(),
            Some("23505"),
            "Error should be unique constraint violation"
        );
    } else {
        panic!("Expected database error for unique constraint violation");
    }
}

#[tokio::test]
#[serial]
async fn test_invoice_attempt_sequence() {
    let pool = setup_pool().await;
    let app_id = "test_app";

    // Cleanup any existing test data
    cleanup_test_data(&pool, app_id).await;

    // Create test invoice
    let invoice_id = create_test_invoice(&pool, app_id).await;

    // Insert multiple attempts with different attempt_no
    for attempt_no in 0..3 {
        let result = sqlx::query(
            r#"
            INSERT INTO ar_invoice_attempts (
                app_id, invoice_id, attempt_no, status
            ) VALUES ($1, $2, $3, 'attempting')
            "#,
        )
        .bind(app_id)
        .bind(invoice_id)
        .bind(attempt_no)
        .execute(&pool)
        .await;

        assert!(
            result.is_ok(),
            "Attempt {} insertion should succeed",
            attempt_no
        );
    }

    // Verify all 3 attempts were created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoice_attempts WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(app_id)
    .bind(invoice_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(count, 3, "Should have exactly 3 attempts");
}

#[tokio::test]
#[serial]
async fn test_invoice_attempt_different_invoices() {
    let pool = setup_pool().await;
    let app_id = "test_app";

    // Cleanup any existing test data
    cleanup_test_data(&pool, app_id).await;

    // Create two test invoices
    let invoice_id_1 = create_test_invoice(&pool, app_id).await;
    let invoice_id_2 = create_test_invoice(&pool, app_id).await;

    // Same attempt_no for different invoices should succeed
    for invoice_id in &[invoice_id_1, invoice_id_2] {
        let result = sqlx::query(
            r#"
            INSERT INTO ar_invoice_attempts (
                app_id, invoice_id, attempt_no, status
            ) VALUES ($1, $2, 0, 'attempting')
            "#,
        )
        .bind(app_id)
        .bind(invoice_id)
        .execute(&pool)
        .await;

        assert!(
            result.is_ok(),
            "Attempt 0 for invoice {} should succeed",
            invoice_id
        );
    }

    // Verify both attempts were created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoice_attempts WHERE app_id = $1 AND attempt_no = 0",
    )
    .bind(app_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        count, 2,
        "Should have 2 attempts (one per invoice) with attempt_no=0"
    );
}

#[tokio::test]
#[serial]
async fn test_invoice_attempt_status_enum() {
    let pool = setup_pool().await;
    let app_id = "test_app";

    // Cleanup any existing test data
    cleanup_test_data(&pool, app_id).await;

    let invoice_id = create_test_invoice(&pool, app_id).await;

    // Test all valid status values
    let statuses = vec!["attempting", "succeeded", "failed_retry", "failed_final"];

    for (idx, status) in statuses.iter().enumerate() {
        let result = sqlx::query(
            r#"
            INSERT INTO ar_invoice_attempts (
                app_id, invoice_id, attempt_no, status
            ) VALUES ($1, $2, $3, $4::ar_invoice_attempt_status)
            "#,
        )
        .bind(app_id)
        .bind(invoice_id)
        .bind(idx as i32)
        .bind(status)
        .execute(&pool)
        .await;

        assert!(
            result.is_ok(),
            "Status '{}' should be valid. Error: {:?}",
            status,
            result.err()
        );
    }

    // Test invalid status should fail
    let result = sqlx::query(
        r#"
        INSERT INTO ar_invoice_attempts (
            app_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, 99, 'invalid_status'::ar_invoice_attempt_status)
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "Invalid status should fail enum constraint"
    );
}

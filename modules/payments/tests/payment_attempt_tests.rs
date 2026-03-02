// Phase 15: Payment Attempt Ledger Tests
// Tests verify UNIQUE constraint enforcement, UNKNOWN protocol support, and attempt tracking

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_test_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

#[tokio::test]
#[serial]
async fn test_payment_attempt_unique_constraint() {
    let pool = setup_test_db().await;

    let app_id = "test_app";
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv_{}", Uuid::new_v4());

    // First attempt should succeed
    let result = sqlx::query(
        r#"
        INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 0, 'attempting')
        "#,
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(&invoice_id)
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "First attempt insertion should succeed");

    // Duplicate attempt should fail (UNIQUE constraint)
    let result = sqlx::query(
        r#"
        INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 0, 'attempting')
        "#,
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(&invoice_id)
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

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[serial]
async fn test_payment_attempt_sequence() {
    let pool = setup_test_db().await;

    let app_id = "test_app";
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv_{}", Uuid::new_v4());

    // Insert multiple attempts with different attempt_no
    for attempt_no in 0..3 {
        let result = sqlx::query(
            r#"
            INSERT INTO payment_attempts (
                app_id, payment_id, invoice_id, attempt_no, status
            ) VALUES ($1, $2, $3, $4, 'attempting')
            "#,
        )
        .bind(app_id)
        .bind(payment_id)
        .bind(&invoice_id)
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
        "SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(app_id)
    .bind(payment_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(count, 3, "Should have exactly 3 attempts");

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[serial]
async fn test_payment_attempt_different_payments() {
    let pool = setup_test_db().await;

    let app_id = "test_app";
    let payment_id_1 = Uuid::new_v4();
    let payment_id_2 = Uuid::new_v4();
    let invoice_id = format!("inv_{}", Uuid::new_v4());

    // Same attempt_no for different payments should succeed
    for payment_id in &[payment_id_1, payment_id_2] {
        let result = sqlx::query(
            r#"
            INSERT INTO payment_attempts (
                app_id, payment_id, invoice_id, attempt_no, status
            ) VALUES ($1, $2, $3, 0, 'attempting')
            "#,
        )
        .bind(app_id)
        .bind(payment_id)
        .bind(&invoice_id)
        .execute(&pool)
        .await;

        assert!(
            result.is_ok(),
            "Attempt 0 for payment {} should succeed",
            payment_id
        );
    }

    // Verify both attempts were created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1 AND attempt_no = 0",
    )
    .bind(app_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to count attempts");

    assert_eq!(
        count, 2,
        "Should have 2 attempts (one per payment) with attempt_no=0"
    );

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[serial]
async fn test_payment_attempt_status_enum() {
    let pool = setup_test_db().await;

    let app_id = "test_app";
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv_{}", Uuid::new_v4());

    // Test all valid status values including UNKNOWN (Phase 15 protocol)
    let statuses = vec![
        "attempting",
        "succeeded",
        "failed_retry",
        "failed_final",
        "unknown", // Phase 15 UNKNOWN protocol
    ];

    for (idx, status) in statuses.iter().enumerate() {
        let result = sqlx::query(
            r#"
            INSERT INTO payment_attempts (
                app_id, payment_id, invoice_id, attempt_no, status
            ) VALUES ($1, $2, $3, $4, $5::payment_attempt_status)
            "#,
        )
        .bind(app_id)
        .bind(payment_id)
        .bind(&invoice_id)
        .bind(idx as i32)
        .bind(status)
        .execute(&pool)
        .await;

        assert!(result.is_ok(), "Status '{}' should be valid", status);
    }

    // Test invalid status should fail
    let result = sqlx::query(
        r#"
        INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 99, 'invalid_status'::payment_attempt_status)
        "#,
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(&invoice_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "Invalid status should fail enum constraint"
    );

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[serial]
async fn test_payment_attempt_unknown_protocol() {
    let pool = setup_test_db().await;

    let app_id = "test_app";
    let payment_id = Uuid::new_v4();
    let invoice_id = format!("inv_{}", Uuid::new_v4());

    // Create an attempt with UNKNOWN status
    let result = sqlx::query(
        r#"
        INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 0, 'unknown'::payment_attempt_status)
        "#,
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(&invoice_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "UNKNOWN status insertion should succeed (Phase 15 protocol)"
    );

    // Verify the attempt was created with UNKNOWN status
    let status: String = sqlx::query_scalar(
        "SELECT status::text FROM payment_attempts WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(app_id)
    .bind(payment_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch attempt status");

    assert_eq!(
        status, "unknown",
        "Attempt should have UNKNOWN status for reconciliation workflow"
    );

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

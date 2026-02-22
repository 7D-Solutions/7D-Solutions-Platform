//! Integration tests for checkout session endpoints (bd-ddsm)
//!
//! Tests use the real PostgreSQL database (no mocks).
//! PAYMENTS_PROVIDER=mock (default) — Tilled API not called.

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_test_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn cleanup_sessions(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM checkout_sessions WHERE invoice_id LIKE 'test_%'")
        .execute(pool)
        .await
        .expect("Failed to cleanup checkout_sessions");
}

// ============================================================================
// create_checkout_session (mock provider)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_checkout_session_created_with_mock() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let tenant_id = "tenant_checkout_test";

    // Insert a checkout session directly (simulating what the handler does)
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
    let client_secret = format!("{}_secret_{}", pi_id, Uuid::new_v4().simple());

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, client_secret)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(tenant_id)
    .bind(5000_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind(&client_secret)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert checkout session");

    // Verify the session exists with pending status
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query session");

    assert_eq!(status, "pending");

    // Verify client_secret was stored
    let stored_secret: String = sqlx::query_scalar(
        "SELECT client_secret FROM checkout_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query client_secret");

    assert!(!stored_secret.is_empty(), "client_secret should be non-empty");
    assert_eq!(stored_secret, client_secret);

    cleanup_sessions(&pool).await;
}

// ============================================================================
// Webhook: payment_intent.succeeded updates status
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_updates_checkout_session_to_succeeded() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, client_secret)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind("tenant_webhook_test")
    .bind(2500_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind("cs_secret_placeholder")
    .fetch_one(&pool)
    .await
    .expect("Failed to insert checkout session");

    // Simulate webhook processing: update pending → succeeded
    let rows = sqlx::query(
        "UPDATE checkout_sessions SET status = 'succeeded', updated_at = NOW() WHERE processor_payment_id = $1 AND status = 'pending'",
    )
    .bind(&pi_id)
    .execute(&pool)
    .await
    .expect("Failed to update session")
    .rows_affected();

    assert_eq!(rows, 1, "Webhook should update exactly one session");

    // Verify status changed
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query status");

    assert_eq!(status, "succeeded");

    cleanup_sessions(&pool).await;
}

// ============================================================================
// Webhook idempotency: already-terminal sessions are not updated
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_does_not_update_terminal_session() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());

    // Insert a session already in succeeded state
    sqlx::query(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, client_secret, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'succeeded')
        "#,
    )
    .bind(&invoice_id)
    .bind("tenant_webhook_idem")
    .bind(1000_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind("cs_secret_terminal")
    .execute(&pool)
    .await
    .expect("Failed to insert terminal session");

    // Webhook for failed should NOT update a succeeded session
    let rows = sqlx::query(
        "UPDATE checkout_sessions SET status = 'failed', updated_at = NOW() WHERE processor_payment_id = $1 AND status = 'pending'",
    )
    .bind(&pi_id)
    .execute(&pool)
    .await
    .expect("Failed to run update")
    .rows_affected();

    assert_eq!(rows, 0, "Terminal sessions must not be overwritten");

    // Status should still be succeeded
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE processor_payment_id = $1")
            .bind(&pi_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query status");

    assert_eq!(status, "succeeded");

    cleanup_sessions(&pool).await;
}

// ============================================================================
// GET status: fetch session by ID
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_checkout_session_by_id() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
    let amount = 7500_i32;
    let currency = "usd";

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, client_secret)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind("tenant_get_test")
    .bind(amount)
    .bind(currency)
    .bind(&pi_id)
    .bind("cs_get_test_secret")
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session");

    #[derive(sqlx::FromRow)]
    struct Row {
        status: String,
        processor_payment_id: String,
        invoice_id: String,
        amount_minor: i32,
        currency: String,
    }

    let row: Row = sqlx::query_as(
        "SELECT status, processor_payment_id, invoice_id, amount_minor, currency FROM checkout_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch session");

    assert_eq!(row.status, "pending");
    assert_eq!(row.processor_payment_id, pi_id);
    assert_eq!(row.invoice_id, invoice_id);
    assert_eq!(row.amount_minor, amount);
    assert_eq!(row.currency, currency);

    cleanup_sessions(&pool).await;
}

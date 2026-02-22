//! Integration tests for checkout session endpoints (bd-ddsm, bd-x0rt)
//!
//! Tests use the real PostgreSQL database (no mocks).
//! PAYMENTS_PROVIDER=mock (default) — Tilled API not called.
//!
//! Status state machine: created → presented → completed | failed | canceled | expired

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

/// Insert a session with the given status for testing purposes.
async fn insert_session(pool: &sqlx::PgPool, invoice_id: &str, status: &str) -> (Uuid, String) {
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, client_secret, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(invoice_id)
    .bind("tenant_test")
    .bind(2500_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind("cs_secret_test")
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to insert checkout session");
    (session_id, pi_id)
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

    // New state machine: initial status is 'created' (not 'pending')
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query session");

    assert_eq!(status, "created");

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
// Present transition: created → presented (idempotent)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_present_transition_created_to_presented() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, _) = insert_session(&pool, &invoice_id, "created").await;

    // First present call: should update 1 row
    let rows = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'presented', presented_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'created'",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("Failed to present session")
    .rows_affected();

    assert_eq!(rows, 1, "First present should update exactly 1 row");

    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query status");
    assert_eq!(status, "presented");

    // presented_at should be set
    let presented_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT presented_at FROM checkout_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query presented_at");
    assert!(presented_at.is_some(), "presented_at must be set after present");

    cleanup_sessions(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_present_transition_is_idempotent() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, _) = insert_session(&pool, &invoice_id, "created").await;

    // First call: transitions to presented
    let rows1 = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'presented', presented_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'created'",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("Failed to present session")
    .rows_affected();

    assert_eq!(rows1, 1);

    // Second call: idempotent, 0 rows affected (already presented)
    let rows2 = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'presented', presented_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'created'",
    )
    .bind(session_id)
    .execute(&pool)
    .await
    .expect("Failed to re-present session")
    .rows_affected();

    assert_eq!(rows2, 0, "Second present on already-presented session must be a no-op");

    // Status remains 'presented'
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query status");
    assert_eq!(status, "presented", "Status must remain presented after idempotent call");

    cleanup_sessions(&pool).await;
}

// ============================================================================
// Webhook: payment_intent.succeeded → completed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_updates_checkout_session_to_completed() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, pi_id) = insert_session(&pool, &invoice_id, "presented").await;

    // Simulate webhook: presented → completed
    let rows = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'completed', updated_at = NOW() \
         WHERE processor_payment_id = $1 AND status IN ('created', 'presented')",
    )
    .bind(&pi_id)
    .execute(&pool)
    .await
    .expect("Failed to update session")
    .rows_affected();

    assert_eq!(rows, 1, "Webhook should update exactly one session");

    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query status");

    assert_eq!(status, "completed");

    cleanup_sessions(&pool).await;
}

// ============================================================================
// Webhook idempotency: replay does not duplicate mutations
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_replay_is_idempotent() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, pi_id) = insert_session(&pool, &invoice_id, "presented").await;

    // First webhook delivery: presented → completed (1 row affected)
    let rows1 = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'completed', updated_at = NOW() \
         WHERE processor_payment_id = $1 AND status IN ('created', 'presented')",
    )
    .bind(&pi_id)
    .execute(&pool)
    .await
    .expect("Failed on first webhook update")
    .rows_affected();

    assert_eq!(rows1, 1, "First webhook delivery must update exactly 1 row");

    // Second delivery (replay of same event): 0 rows affected — idempotent no-op
    let rows2 = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'completed', updated_at = NOW() \
         WHERE processor_payment_id = $1 AND status IN ('created', 'presented')",
    )
    .bind(&pi_id)
    .execute(&pool)
    .await
    .expect("Failed on replayed webhook update")
    .rows_affected();

    assert_eq!(rows2, 0, "Webhook replay must NOT mutate an already-terminal session");

    // Session is still 'completed' — not corrupted by replay
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query final status");

    assert_eq!(status, "completed", "Status must remain completed after webhook replay");

    cleanup_sessions(&pool).await;
}

// ============================================================================
// Webhook idempotency: terminal sessions not overwritten by any webhook type
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_does_not_update_terminal_session() {
    let pool = setup_test_db().await;
    cleanup_sessions(&pool).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (_, pi_id) = insert_session(&pool, &invoice_id, "completed").await;

    // A failed-event webhook must NOT overwrite a completed session
    let rows = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'failed', updated_at = NOW() \
         WHERE processor_payment_id = $1 AND status IN ('created', 'presented')",
    )
    .bind(&pi_id)
    .execute(&pool)
    .await
    .expect("Failed to run update")
    .rows_affected();

    assert_eq!(rows, 0, "Terminal sessions must not be overwritten");

    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE processor_payment_id = $1")
            .bind(&pi_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query status");

    assert_eq!(status, "completed");

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

    // New state machine: initial status is 'created'
    assert_eq!(row.status, "created");
    assert_eq!(row.processor_payment_id, pi_id);
    assert_eq!(row.invoice_id, invoice_id);
    assert_eq!(row.amount_minor, amount);
    assert_eq!(row.currency, currency);

    cleanup_sessions(&pool).await;
}

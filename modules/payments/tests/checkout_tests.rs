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

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");

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

fn unique_tenant_id() -> String {
    format!("tenant_checkout_{}", Uuid::new_v4().simple())
}

async fn cleanup_sessions(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM checkout_sessions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup checkout_sessions");
}

/// Insert a session with the given status for testing purposes.
async fn insert_session(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    invoice_id: &str,
    status: &str,
) -> (Uuid, String) {
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
    let idem_key = format!("test_idem_{}", Uuid::new_v4().simple());
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id,
             idempotency_key, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(invoice_id)
    .bind(tenant_id)
    .bind(2500_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind(&idem_key)
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
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());

    // Insert a checkout session directly (simulating what the handler does)
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());

    let idem_key = format!("idem_{}", Uuid::new_v4().simple());
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(5000_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert checkout session");

    // New state machine: initial status is 'created' (not 'pending')
    let status: String = sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query session");

    assert_eq!(status, "created");

    cleanup_sessions(&pool, &tenant_id).await;
}

// ============================================================================
// Present transition: created → presented (idempotent)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_present_transition_created_to_presented() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, _) = insert_session(&pool, &tenant_id, &invoice_id, "created").await;

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

    let status: String = sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query status");
    assert_eq!(status, "presented");

    // presented_at should be set
    let presented_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT presented_at FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to query presented_at");
    assert!(
        presented_at.is_some(),
        "presented_at must be set after present"
    );

    cleanup_sessions(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_present_transition_is_idempotent() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, _) = insert_session(&pool, &tenant_id, &invoice_id, "created").await;

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

    assert_eq!(
        rows2, 0,
        "Second present on already-presented session must be a no-op"
    );

    // Status remains 'presented'
    let status: String = sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query status");
    assert_eq!(
        status, "presented",
        "Status must remain presented after idempotent call"
    );

    cleanup_sessions(&pool, &tenant_id).await;
}

// ============================================================================
// Webhook: payment_intent.succeeded → completed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_updates_checkout_session_to_completed() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, pi_id) = insert_session(&pool, &tenant_id, &invoice_id, "presented").await;

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

    let status: String = sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query status");

    assert_eq!(status, "completed");

    cleanup_sessions(&pool, &tenant_id).await;
}

// ============================================================================
// Webhook idempotency: replay does not duplicate mutations
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_replay_is_idempotent() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (session_id, pi_id) = insert_session(&pool, &tenant_id, &invoice_id, "presented").await;

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

    assert_eq!(
        rows2, 0,
        "Webhook replay must NOT mutate an already-terminal session"
    );

    // Session is still 'completed' — not corrupted by replay
    let status: String = sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query final status");

    assert_eq!(
        status, "completed",
        "Status must remain completed after webhook replay"
    );

    cleanup_sessions(&pool, &tenant_id).await;
}

// ============================================================================
// Webhook idempotency: terminal sessions not overwritten by any webhook type
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_does_not_update_terminal_session() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let (_, pi_id) = insert_session(&pool, &tenant_id, &invoice_id, "completed").await;

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

    cleanup_sessions(&pool, &tenant_id).await;
}

// ============================================================================
// GET status: fetch session by ID
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_checkout_session_by_id() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
    let amount = 7500_i64;
    let currency = "usd";

    let idem_key = format!("idem_{}", Uuid::new_v4().simple());
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(amount)
    .bind(currency)
    .bind(&pi_id)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session");

    #[derive(sqlx::FromRow)]
    struct Row {
        status: String,
        processor_payment_id: String,
        invoice_id: String,
        amount_minor: i64,
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

    cleanup_sessions(&pool, &tenant_id).await;
}

// ============================================================================
// Idempotency: same (tenant, idempotency_key) returns existing session
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency_explicit_key_returns_existing() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let idem_key = format!("idem_{}", Uuid::new_v4().simple());
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
    let secret = format!("{}_secret_test", pi_id);

    // Insert first session with explicit idempotency_key
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id,
             client_secret, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(5000_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind(&secret)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert first session");

    // Lookup by (tenant_id, idempotency_key) — simulates handler idempotency check
    let existing: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
        "SELECT id, processor_payment_id, client_secret \
         FROM checkout_sessions \
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(&tenant_id)
    .bind(&idem_key)
    .fetch_optional(&pool)
    .await
    .expect("Failed to query existing session");

    assert!(
        existing.is_some(),
        "Must find existing session by idempotency_key"
    );
    let (found_id, found_pi, found_secret) = existing.unwrap();
    assert_eq!(found_id, session_id, "Must return same session_id");
    assert_eq!(found_pi, pi_id, "Must return same payment_intent_id");
    assert_eq!(
        found_secret.as_deref(),
        Some(secret.as_str()),
        "Must return stored client_secret"
    );

    cleanup_sessions(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_idempotency_invoice_id_as_natural_key() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());

    // Insert session using invoice_id as idempotency_key (handler default)
    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id,
             client_secret, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(3000_i32)
    .bind("usd")
    .bind(&pi_id)
    .bind("secret_abc")
    .bind(&invoice_id) // invoice_id used as idempotency_key
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session");

    // Second insert with same (tenant_id, invoice_id) as key must fail (UNIQUE violation)
    let dup_result: Result<Uuid, _> = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id,
             client_secret, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(3000_i32)
    .bind("usd")
    .bind("another_pi")
    .bind("secret_xyz")
    .bind(&invoice_id) // same key
    .fetch_one(&pool)
    .await;

    assert!(
        dup_result.is_err(),
        "Duplicate (tenant_id, idempotency_key) must be rejected by UNIQUE constraint"
    );
    let err_msg = dup_result.unwrap_err().to_string();
    assert!(
        err_msg.contains("duplicate key")
            || err_msg.contains("uq_checkout_sessions_tenant_idem_key"),
        "Error must reference the unique constraint, got: {err_msg}"
    );

    // Original session unchanged
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM checkout_sessions WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(&tenant_id)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to count sessions");
    assert_eq!(count, 1, "Exactly one session must exist for the key");

    // Fetch by idempotency_key returns original
    let found_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM checkout_sessions WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(&tenant_id)
    .bind(&invoice_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch session");
    assert_eq!(found_id, session_id);

    cleanup_sessions(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_idempotency_different_keys_create_separate_sessions() {
    let pool = setup_test_db().await;
    let tenant_id = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_id).await;

    let invoice_id = format!("test_inv_{}", Uuid::new_v4().simple());
    let key_a = format!("key_a_{}", Uuid::new_v4().simple());
    let key_b = format!("key_b_{}", Uuid::new_v4().simple());

    let session_a: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(1000_i32)
    .bind("usd")
    .bind("pi_a")
    .bind(&key_a)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session A");

    let session_b: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(&invoice_id)
    .bind(&tenant_id)
    .bind(1000_i32)
    .bind("usd")
    .bind("pi_b")
    .bind(&key_b)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session B");

    assert_ne!(
        session_a, session_b,
        "Different keys must produce different sessions"
    );

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM checkout_sessions WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("Failed to count");
    assert_eq!(count, 2);

    cleanup_sessions(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_idempotency_same_key_different_tenants_allowed() {
    let pool = setup_test_db().await;
    let tenant_a = unique_tenant_id();
    let tenant_b = unique_tenant_id();
    cleanup_sessions(&pool, &tenant_a).await;
    cleanup_sessions(&pool, &tenant_b).await;

    let idem_key = format!("shared_key_{}", Uuid::new_v4().simple());

    let session_a: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("inv_a")
    .bind(&tenant_a)
    .bind(1000_i32)
    .bind("usd")
    .bind("pi_a")
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session for tenant A");

    let session_b: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("inv_b")
    .bind(&tenant_b)
    .bind(2000_i32)
    .bind("usd")
    .bind("pi_b")
    .bind(&idem_key) // same key, different tenant
    .fetch_one(&pool)
    .await
    .expect("Failed to insert session for tenant B — UNIQUE should be scoped by tenant");

    assert_ne!(session_a, session_b);

    cleanup_sessions(&pool, &tenant_a).await;
    cleanup_sessions(&pool, &tenant_b).await;
}

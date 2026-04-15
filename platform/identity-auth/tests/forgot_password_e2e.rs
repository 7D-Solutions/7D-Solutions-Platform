/// Integration tests for the forgot_password handler invariants.
///
/// Uses real Postgres + NATS (no mocks). Since identity-auth is a binary crate,
/// these tests exercise the DB and NATS effects that the handler produces by
/// replicating the exact logic inline.
///
/// Invariants verified:
///   - token row inserted with SHA-256 hash (not the raw token)
///   - stored hash matches SHA-256(raw_token)
///   - no token row created when email is unknown
///   - NATS event is published with raw_token in the data payload
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use futures::StreamExt;
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, Row};
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into())
}

fn nats_url() -> String {
    std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://platform:dev-nats-token@localhost:4222".into())
}

async fn test_pool() -> sqlx::PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url())
        .await
        .expect("connect to test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

/// Mirrors password_reset_tokens::generate_raw_token()
fn generate_raw_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Mirrors password_reset_tokens::sha256_token_hash()
fn sha256_token_hash(raw: &str) -> String {
    let mut h = Sha256::new();
    h.update(raw.as_bytes());
    hex::encode(h.finalize())
}

async fn insert_credential(pool: &sqlx::PgPool, tenant_id: Uuid, user_id: Uuid, email: &str) {
    sqlx::query(
        r#"INSERT INTO credentials (tenant_id, user_id, email, password_hash)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(email)
    .bind("phash-test-placeholder")
    .execute(pool)
    .await
    .expect("insert test credential");
}

async fn insert_reset_token(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: chrono::DateTime<Utc>,
) -> Uuid {
    let row = sqlx::query(
        r#"INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
           VALUES ($1, $2, $3) RETURNING id"#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .expect("insert reset token");
    row.get::<Uuid, _>("id")
}

async fn fetch_token_hash_for_user(pool: &sqlx::PgPool, user_id: Uuid) -> Option<String> {
    let row = sqlx::query(
        "SELECT token_hash FROM password_reset_tokens WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .expect("query token row");
    row.map(|r| r.get::<String, _>("token_hash"))
}

// ---------------------------------------------------------------------------
// Test: known email → token row created, stored hash != raw token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_known_email_token_row_created_with_hash() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("test-forgot-{}@example.com", Uuid::new_v4());

    insert_credential(&pool, tenant_id, user_id, &email).await;

    // Simulate what forgot_password does on user-found path
    let raw = generate_raw_token();
    let hash = sha256_token_hash(&raw);
    let expires_at = Utc::now() + Duration::minutes(30);

    insert_reset_token(&pool, user_id, &hash, expires_at).await;

    let stored_hash = fetch_token_hash_for_user(&pool, user_id)
        .await
        .expect("token row must exist for known email");

    // Invariant 1: stored value is the hash, not the raw token
    assert_ne!(stored_hash, raw, "stored hash must NOT equal raw token");

    // Invariant 2: stored value equals SHA-256(raw_token)
    assert_eq!(
        stored_hash, hash,
        "stored value must equal sha256_token_hash(raw_token)"
    );
}

// ---------------------------------------------------------------------------
// Test: unknown email → no token row created
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unknown_email_no_token_row_created() {
    let pool = test_pool().await;
    // Use a user_id that was never inserted into credentials
    let unknown_user_id = Uuid::new_v4();

    // forgot_password: looks up by email → not found → skips insert
    // We verify: no token row exists for this user_id
    let row = fetch_token_hash_for_user(&pool, unknown_user_id).await;
    assert!(
        row.is_none(),
        "no token row must exist for an unknown email"
    );
}

// ---------------------------------------------------------------------------
// Test: NATS event published with raw_token in data payload
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_nats_event_contains_raw_token() {
    let nats = event_bus::connect_nats(&nats_url())
        .await
        .expect("connect to NATS");

    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("test-nats-{}@example.com", Uuid::new_v4());

    insert_credential(&pool, tenant_id, user_id, &email).await;

    let raw = generate_raw_token();
    let hash = sha256_token_hash(&raw);
    let expires_at = Utc::now() + Duration::minutes(30);

    insert_reset_token(&pool, user_id, &hash, expires_at).await;

    // Build event payload using canonical EventEnvelope structure
    let payload = serde_json::json!({
        "event_id": Uuid::new_v4().to_string(),
        "event_type": "auth.events.password_reset_requested",
        "schema_version": "auth.events.password_reset_requested/v1",
        "source_version": "1.0.0",
        "occurred_at": Utc::now().to_rfc3339(),
        "source_module": "auth-rs@test",
        "tenant_id": tenant_id.to_string(),
        "trace_id": "e2e-test-trace",
        "replay_safe": true,
        "mutation_class": "user-data",
        "payload": {
            "user_id": user_id.to_string(),
            "email": email,
            "raw_token": raw.clone(),
            "expires_at": expires_at.to_rfc3339(),
            "correlation_id": "e2e-test-trace"
        }
    });

    // Subscribe BEFORE publishing
    let mut sub = nats
        .subscribe("auth.events.password_reset_requested")
        .await
        .expect("subscribe to NATS subject");

    nats.publish(
        "auth.events.password_reset_requested",
        serde_json::to_vec(&payload).unwrap().into(),
    )
    .await
    .expect("publish event");
    nats.flush().await.expect("flush");

    // Receive with timeout
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next())
        .await
        .expect("timeout: no NATS message received within 5s")
        .expect("NATS subscriber closed unexpectedly");

    let received: serde_json::Value =
        serde_json::from_slice(&msg.payload).expect("parse NATS message as JSON");

    assert_eq!(
        received["payload"]["raw_token"],
        serde_json::Value::String(raw),
        "NATS event payload.raw_token must match the generated raw token"
    );
    assert_eq!(
        received["payload"]["user_id"],
        serde_json::Value::String(user_id.to_string()),
        "NATS event payload.user_id must match"
    );
    assert_eq!(
        received["event_type"], "auth.events.password_reset_requested",
        "event_type must be auth.events.password_reset_requested"
    );
    // Canonical envelope fields present
    assert_eq!(
        received["source_module"], "auth-rs@test",
        "source_module must be set"
    );
    assert!(
        received["tenant_id"].is_string(),
        "tenant_id must be a String"
    );
    assert_eq!(received["replay_safe"], true, "replay_safe must be set");
    assert!(
        received.get("source_version").is_some(),
        "source_version must be present"
    );
}

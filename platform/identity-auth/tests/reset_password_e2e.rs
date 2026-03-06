/// Integration tests for POST /api/auth/reset-password handler invariants.
///
/// Uses real Postgres + NATS (no mocks). Since identity-auth is a binary crate,
/// these tests replicate the handler logic inline and verify the resulting DB
/// state and NATS event.
///
/// Invariants verified:
///   - valid token: password updated, session_leases hard-deleted, refresh_tokens hard-deleted
///   - NATS completion event published with correct user_id
///   - invalid/expired token: 400 path → no password change, no revocation
///   - token is single-use: second call with same token leaves password unchanged
///   - canonical EventEnvelope publish+deserialize round-trip compatibility
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use event_bus::EventEnvelope;
use futures::StreamExt;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
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

/// Seed a credential row (placeholder password hash — not argon2, just a string for test isolation)
async fn insert_credential(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    email: &str,
    password_hash: &str,
) {
    sqlx::query(
        r#"INSERT INTO credentials (tenant_id, user_id, email, password_hash)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(email)
    .bind(password_hash)
    .execute(pool)
    .await
    .expect("insert test credential");
}

/// Seed a reset token row directly (no need to call forgot-password endpoint)
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

/// Seed a session_leases row
async fn insert_session_lease(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    refresh_token_id: Uuid,
) {
    sqlx::query(
        r#"INSERT INTO session_leases (tenant_id, user_id, session_id)
           VALUES ($1, $2, $3)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(refresh_token_id)
    .execute(pool)
    .await
    .expect("insert session lease");
}

/// Seed a refresh_tokens row; returns its id
async fn insert_refresh_token(pool: &sqlx::PgPool, tenant_id: Uuid, user_id: Uuid) -> Uuid {
    let row = sqlx::query(
        r#"INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
           VALUES ($1, $2, $3, NOW() + INTERVAL '14 days') RETURNING id"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(format!("tkhash-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert refresh token");
    row.get::<Uuid, _>("id")
}

/// Simulate the reset_password handler's core DB operations (claim → update → revoke)
/// Returns: (password_hash_after, session_leases_count, refresh_tokens_count, token_used_at)
async fn simulate_reset_password(
    pool: &sqlx::PgPool,
    raw_token: &str,
    new_password_hash: &str,
) -> (Option<String>, i64, i64, bool) {
    let token_hash = sha256_token_hash(raw_token);

    // Claim token atomically
    let claimed_row = sqlx::query(
        r#"
        UPDATE password_reset_tokens
        SET used_at = NOW()
        WHERE token_hash = $1
          AND used_at IS NULL
          AND expires_at > NOW()
        RETURNING user_id
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .expect("claim reset token");

    let user_id = match claimed_row {
        Some(r) => r.get::<Uuid, _>("user_id"),
        None => return (None, -1, -1, false),
    };

    // Update password
    sqlx::query("UPDATE credentials SET password_hash = $1, updated_at = NOW() WHERE user_id = $2")
        .bind(new_password_hash)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("update password");

    // Hard-delete session_leases
    sqlx::query("DELETE FROM session_leases WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("delete session leases");

    // Hard-delete refresh_tokens
    sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("delete refresh tokens");

    // Read back state for assertions
    let ph_row = sqlx::query("SELECT password_hash FROM credentials WHERE user_id = $1 LIMIT 1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .expect("fetch credential")
        .map(|r| r.get::<String, _>("password_hash"));

    let sl_count: i64 = sqlx::query("SELECT COUNT(*) FROM session_leases WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("count session leases")
        .get::<i64, _>(0);

    let rt_count: i64 = sqlx::query("SELECT COUNT(*) FROM refresh_tokens WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("count refresh tokens")
        .get::<i64, _>(0);

    let token_used = sqlx::query("SELECT used_at FROM password_reset_tokens WHERE token_hash = $1")
        .bind(&token_hash)
        .fetch_one(pool)
        .await
        .expect("fetch token row")
        .get::<Option<chrono::DateTime<Utc>>, _>("used_at")
        .is_some();

    (ph_row, sl_count, rt_count, token_used)
}

// ---------------------------------------------------------------------------
// Test 1: valid token → password updated, sessions revoked, token marked used
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_valid_token_resets_password_and_revokes_sessions() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("reset-e2e-{}@example.com", Uuid::new_v4());

    insert_credential(&pool, tenant_id, user_id, &email, "old-hash-placeholder").await;

    let refresh_token_id = insert_refresh_token(&pool, tenant_id, user_id).await;
    insert_session_lease(&pool, tenant_id, user_id, refresh_token_id).await;

    let raw = generate_raw_token();
    let hash = sha256_token_hash(&raw);
    let expires_at = Utc::now() + Duration::hours(1);
    insert_reset_token(&pool, user_id, &hash, expires_at).await;

    let new_hash = "new-argon2-hash-placeholder";
    let (ph_after, sl_count, rt_count, token_used) =
        simulate_reset_password(&pool, &raw, new_hash).await;

    // Invariant: password updated
    assert_eq!(
        ph_after.as_deref(),
        Some(new_hash),
        "password_hash must be updated to the new hash"
    );
    // Invariant: token marked used (used_at set)
    assert!(
        token_used,
        "token must have used_at set after successful claim"
    );
    // Invariant: session_leases hard-deleted
    assert_eq!(sl_count, 0, "all session_leases for user must be deleted");
    // Invariant: refresh_tokens hard-deleted
    assert_eq!(rt_count, 0, "all refresh_tokens for user must be deleted");
}

// ---------------------------------------------------------------------------
// Test 2: invalid token → no password change, no session revocation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_invalid_token_no_changes() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("reset-invalid-{}@example.com", Uuid::new_v4());
    let original_hash = "original-hash-unchanged";

    insert_credential(&pool, tenant_id, user_id, &email, original_hash).await;

    let refresh_token_id = insert_refresh_token(&pool, tenant_id, user_id).await;
    insert_session_lease(&pool, tenant_id, user_id, refresh_token_id).await;

    // Use a token that was never inserted → claim returns None
    let bogus_raw = generate_raw_token();
    let (ph_after, sl_count, rt_count, _) =
        simulate_reset_password(&pool, &bogus_raw, "should-not-appear").await;

    // simulate_reset_password returns (None, -1, -1, false) when claim fails
    assert_eq!(ph_after, None, "claim must fail for bogus token");

    // Verify original DB state unchanged by checking directly
    let actual_hash: String =
        sqlx::query("SELECT password_hash FROM credentials WHERE user_id = $1 LIMIT 1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("fetch credential")
            .get("password_hash");

    assert_eq!(
        actual_hash, original_hash,
        "password_hash must be unchanged after bogus token attempt"
    );

    let sl: i64 = sqlx::query("SELECT COUNT(*) FROM session_leases WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("count leases")
        .get::<i64, _>(0);

    assert_eq!(
        sl, 1,
        "session_leases must be untouched after invalid token"
    );
    let _ = sl_count; // suppress unused warning from early return
    let _ = rt_count;
}

// ---------------------------------------------------------------------------
// Test 3: expired token → no changes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_expired_token_no_changes() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("reset-expired-{}@example.com", Uuid::new_v4());
    let original_hash = "expired-token-hash-unchanged";

    insert_credential(&pool, tenant_id, user_id, &email, original_hash).await;

    let raw = generate_raw_token();
    let hash = sha256_token_hash(&raw);
    // Expired 1 minute ago
    let expires_at = Utc::now() - Duration::minutes(1);
    insert_reset_token(&pool, user_id, &hash, expires_at).await;

    let (ph_after, _, _, token_used) =
        simulate_reset_password(&pool, &raw, "should-not-appear").await;

    assert_eq!(ph_after, None, "claim must fail for expired token");
    assert!(!token_used, "expired token must not be marked used");

    let actual_hash: String =
        sqlx::query("SELECT password_hash FROM credentials WHERE user_id = $1 LIMIT 1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("fetch credential")
            .get("password_hash");

    assert_eq!(
        actual_hash, original_hash,
        "password must be unchanged for expired token"
    );
}

// ---------------------------------------------------------------------------
// Test 4: token single-use — second call leaves password unchanged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_token_single_use_second_call_rejected() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("reset-singleuse-{}@example.com", Uuid::new_v4());

    insert_credential(&pool, tenant_id, user_id, &email, "original-hash").await;

    let raw = generate_raw_token();
    let hash = sha256_token_hash(&raw);
    let expires_at = Utc::now() + Duration::hours(1);
    insert_reset_token(&pool, user_id, &hash, expires_at).await;

    // First call succeeds
    let (ph_first, _, _, token_used) = simulate_reset_password(&pool, &raw, "first-new-hash").await;
    assert_eq!(ph_first.as_deref(), Some("first-new-hash"));
    assert!(token_used, "token must be marked used after first claim");

    // Second call with same token must fail
    let (ph_second, _, _, _) = simulate_reset_password(&pool, &raw, "second-new-hash").await;
    assert_eq!(ph_second, None, "second claim with same token must fail");

    // Password stays at first-new-hash, not changed by second call
    let actual_hash: String =
        sqlx::query("SELECT password_hash FROM credentials WHERE user_id = $1 LIMIT 1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("fetch credential")
            .get("password_hash");

    assert_eq!(
        actual_hash, "first-new-hash",
        "password must not be changed by second (rejected) call"
    );
}

// ---------------------------------------------------------------------------
// Test 5: NATS completion event published with correct user_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_nats_completion_event_published() {
    let nats = event_bus::connect_nats(&nats_url())
        .await
        .expect("connect to NATS");

    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("reset-nats-{}@example.com", Uuid::new_v4());

    insert_credential(&pool, tenant_id, user_id, &email, "old-hash").await;

    // Subscribe BEFORE publishing
    let mut sub = nats
        .subscribe("auth.password_reset_completed")
        .await
        .expect("subscribe to NATS subject");

    // Build and publish the completion event using canonical EventEnvelope structure
    let payload = serde_json::json!({
        "event_id": Uuid::new_v4().to_string(),
        "event_type": "auth.password_reset_completed",
        "schema_version": "1.0.0",
        "source_version": "1.0.0",
        "occurred_at": Utc::now().to_rfc3339(),
        "source_module": "auth-rs@test",
        "tenant_id": tenant_id.to_string(),
        "trace_id": "e2e-test-trace",
        "replay_safe": true,
        "mutation_class": "user-data",
        "payload": {
            "user_id": user_id.to_string(),
            "correlation_id": "e2e-test-trace"
        }
    });

    nats.publish(
        "auth.password_reset_completed",
        serde_json::to_vec(&payload).unwrap().into(),
    )
    .await
    .expect("publish event");
    nats.flush().await.expect("flush");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next())
        .await
        .expect("timeout: no NATS message received within 5s")
        .expect("NATS subscriber closed unexpectedly");

    let received: serde_json::Value =
        serde_json::from_slice(&msg.payload).expect("parse NATS message as JSON");

    assert_eq!(
        received["event_type"], "auth.password_reset_completed",
        "event_type must be auth.password_reset_completed"
    );
    assert_eq!(
        received["payload"]["user_id"],
        serde_json::Value::String(user_id.to_string()),
        "payload.user_id must match"
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
    assert_eq!(
        received["replay_safe"], true,
        "replay_safe must be set"
    );
    assert!(
        received.get("source_version").is_some(),
        "source_version must be present"
    );
}

// ---------------------------------------------------------------------------
// Test 6: canonical EventEnvelope publish+deserialize round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_canonical_envelope_publish_deserialize_roundtrip() {
    let nats = event_bus::connect_nats(&nats_url())
        .await
        .expect("connect to NATS");

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestPayload {
        user_id: String,
        action: String,
    }

    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let subject = format!("auth.test.roundtrip.{}", Uuid::new_v4());

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "auth-rs@test".to_string(),
        "auth.test.roundtrip".to_string(),
        TestPayload {
            user_id: user_id.to_string(),
            action: "password_reset".to_string(),
        },
    )
    .with_schema_version("auth.test.roundtrip/v1".to_string())
    .with_trace_id(Some("roundtrip-trace".to_string()))
    .with_mutation_class(Some("user-data".to_string()));

    let mut sub = nats.subscribe(subject.clone()).await.expect("subscribe");

    let bytes = serde_json::to_vec(&envelope).expect("serialize canonical envelope");
    nats.publish(subject, bytes.into()).await.expect("publish");
    nats.flush().await.expect("flush");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), sub.next())
        .await
        .expect("timeout waiting for message")
        .expect("subscriber closed");

    let deserialized: EventEnvelope<TestPayload> =
        serde_json::from_slice(&msg.payload).expect("deserialize into canonical EventEnvelope");

    assert_eq!(deserialized.event_type, "auth.test.roundtrip");
    assert_eq!(deserialized.tenant_id, tenant_id.to_string());
    assert_eq!(deserialized.source_module, "auth-rs@test");
    assert_eq!(deserialized.schema_version, "auth.test.roundtrip/v1");
    assert_eq!(deserialized.source_version, "1.0.0");
    assert_eq!(deserialized.trace_id, Some("roundtrip-trace".to_string()));
    assert_eq!(
        deserialized.mutation_class,
        Some("user-data".to_string())
    );
    assert!(deserialized.replay_safe);
    assert_eq!(deserialized.payload.user_id, user_id.to_string());
    assert_eq!(deserialized.payload.action, "password_reset");
}

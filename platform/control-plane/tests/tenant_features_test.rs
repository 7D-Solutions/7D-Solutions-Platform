//! Integration tests for GET /api/features (bd-p2jsi).
//!
//! Verifies:
//!   (a) valid token + matching tenant_id → 200 with flags map
//!   (b) valid token + mismatched tenant_id → 403
//!   (c) no token → 401
//!   (d) missing tenant_id param → 400
//!   (e) malformed UUID tenant_id → 400
//!
//! Uses real Postgres (TENANT_REGISTRY_DATABASE_URL) and real RSA keypairs.
//! No mocks.

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use control_plane::routes::build_router;
use control_plane::state::AppState;
use tenant_registry::routes::SummaryState;

// ============================================================================
// Helpers
// ============================================================================

struct TestKeys {
    encoding: EncodingKey,
    verifier: Arc<security::JwtVerifier>,
}

fn make_test_keys() -> TestKeys {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key gen");
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("priv PEM");
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).expect("pub PEM");
    TestKeys {
        encoding: EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key"),
        verifier: Arc::new(security::JwtVerifier::from_public_pem(&pub_pem).expect("JWT verifier")),
    }
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn mint_token(enc: &EncodingKey, tenant_id: Uuid) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["user".into()],
        perms: vec![],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, enc).expect("sign token")
}

async fn test_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    sqlx::pool::PoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

fn build_server(pool: PgPool, verifier: Arc<security::JwtVerifier>) -> TestServer {
    let app_state = Arc::new(AppState::new(pool.clone(), None).with_verifier(verifier));
    let summary_state = Arc::new(SummaryState::new_local(pool));
    let router = build_router(app_state, summary_state);
    TestServer::new(router).expect("build test server")
}

// ============================================================================
// Tests
// ============================================================================

/// (a) Valid token + matching tenant_id → 200 with flags map
#[tokio::test]
async fn tenant_features_valid_token_returns_200() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let tenant_id = Uuid::new_v4();
    let token = mint_token(&keys.encoding, tenant_id);
    let server = build_server(pool.clone(), keys.verifier);

    // Seed a per-tenant flag and a global flag
    sqlx::query(
        "INSERT INTO feature_flags (flag_name, tenant_id, enabled) VALUES ($1, $2, true) \
         ON CONFLICT (flag_name, tenant_id) WHERE tenant_id IS NOT NULL \
         DO UPDATE SET enabled = EXCLUDED.enabled",
    )
    .bind("per_tenant_flag")
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("seed per-tenant flag");

    sqlx::query(
        "INSERT INTO feature_flags (flag_name, tenant_id, enabled) VALUES ($1, NULL, true) \
         ON CONFLICT (flag_name) WHERE tenant_id IS NULL \
         DO UPDATE SET enabled = EXCLUDED.enabled",
    )
    .bind("global_flag_for_features_test")
    .execute(&pool)
    .await
    .expect("seed global flag");

    let resp = server
        .get("/api/features")
        .authorization_bearer(token)
        .add_query_param("tenant_id", tenant_id.to_string())
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert!(body["flags"].is_object(), "flags must be an object");
    assert_eq!(body["flags"]["per_tenant_flag"], true);
    assert_eq!(body["flags"]["global_flag_for_features_test"], true);

    // Cleanup
    sqlx::query("DELETE FROM feature_flags WHERE flag_name = 'per_tenant_flag' AND tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM feature_flags WHERE flag_name = 'global_flag_for_features_test' AND tenant_id IS NULL")
        .execute(&pool)
        .await
        .ok();
}

/// (b) Valid token + mismatched tenant_id → 403
#[tokio::test]
async fn tenant_features_mismatched_tenant_returns_403() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let token_tenant = Uuid::new_v4();
    let other_tenant = Uuid::new_v4();
    let token = mint_token(&keys.encoding, token_tenant);
    let server = build_server(pool, keys.verifier);

    let resp = server
        .get("/api/features")
        .authorization_bearer(token)
        .add_query_param("tenant_id", other_tenant.to_string())
        .await;

    resp.assert_status(StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["error"], "forbidden");
}

/// (c) No token → 401
#[tokio::test]
async fn tenant_features_no_token_returns_401() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let server = build_server(pool, keys.verifier);

    let resp = server
        .get("/api/features")
        .add_query_param("tenant_id", Uuid::new_v4().to_string())
        .await;

    resp.assert_status(StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["error"], "unauthorized");
}

/// (d) Missing tenant_id param → 400
#[tokio::test]
async fn tenant_features_missing_tenant_id_returns_400() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let tenant_id = Uuid::new_v4();
    let token = mint_token(&keys.encoding, tenant_id);
    let server = build_server(pool, keys.verifier);

    let resp = server
        .get("/api/features")
        .authorization_bearer(token)
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json();
    assert!(body["error"].as_str().is_some());
}

/// (e) Malformed UUID tenant_id → 400
#[tokio::test]
async fn tenant_features_malformed_uuid_returns_400() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let tenant_id = Uuid::new_v4();
    let token = mint_token(&keys.encoding, tenant_id);
    let server = build_server(pool, keys.verifier);

    let resp = server
        .get("/api/features")
        .authorization_bearer(token)
        .add_query_param("tenant_id", "not-a-uuid")
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json();
    assert!(body["error"].as_str().is_some());
}

/// schema_version field and /api/schemas/features/v{N} endpoint
///
/// Covers:
///   (f) GET /api/features response includes schema_version == 1
///   (g) GET /api/schemas/features/v1 returns JSON Schema with required fields
///   (h) GET /api/schemas/features/v999 returns 404
#[tokio::test]
async fn features_schema_version() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let tenant_id = Uuid::new_v4();
    let token = mint_token(&keys.encoding, tenant_id);
    let server = build_server(pool, keys.verifier);

    // (f) Features payload carries schema_version = 1
    let resp = server
        .get("/api/features")
        .authorization_bearer(token)
        .add_query_param("tenant_id", tenant_id.to_string())
        .await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["schema_version"].as_u64(),
        Some(1),
        "schema_version must be 1"
    );

    // (g) Schema endpoint returns a valid JSON Schema for v1
    let schema_resp = server.get("/api/schemas/features/v1").await;
    schema_resp.assert_status(StatusCode::OK);
    let schema: serde_json::Value = schema_resp.json();
    assert_eq!(
        schema["$id"].as_str(),
        Some("/api/schemas/features/v1"),
        "schema $id must match endpoint path"
    );
    let required = schema["required"].as_array().expect("required must be array");
    let required_strs: Vec<&str> = required
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        required_strs.contains(&"tenant_id"),
        "schema must require tenant_id"
    );
    assert!(
        required_strs.contains(&"schema_version"),
        "schema must require schema_version"
    );
    assert!(
        required_strs.contains(&"flags"),
        "schema must require flags"
    );
    assert_eq!(
        schema["properties"]["schema_version"]["const"].as_u64(),
        Some(1),
        "schema must constrain schema_version to 1"
    );

    // (h) Unknown schema version returns 404
    let unknown_resp = server.get("/api/schemas/features/v999").await;
    unknown_resp.assert_status(StatusCode::NOT_FOUND);
}

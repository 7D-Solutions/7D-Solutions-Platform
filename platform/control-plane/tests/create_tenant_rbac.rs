//! RBAC integration test for POST /api/control/tenants (bd-505dg).
//!
//! Verifies that:
//!   1. A caller with PLATFORM_TENANTS_CREATE permission receives 202 Accepted.
//!   2. A caller WITHOUT PLATFORM_TENANTS_CREATE receives 403 Forbidden.
//!   3. A caller with no JWT at all receives 401 Unauthorized.
//!
//! Uses real RSA keypairs and a real Postgres database. No mocks.

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
        verifier: Arc::new(
            security::JwtVerifier::from_public_pem(&pub_pem).expect("JWT verifier"),
        ),
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

fn mint_token(enc: &EncodingKey, perms: Vec<String>) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        roles: vec!["platform_admin".into()],
        perms,
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
    PgPool::connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

fn build_authed_server(pool: PgPool, verifier: Arc<security::JwtVerifier>) -> TestServer {
    let app_state = Arc::new(AppState::new(pool.clone(), None).with_verifier(verifier));
    let summary_state = Arc::new(SummaryState::new_local(pool));
    let router = build_router(app_state, summary_state);
    TestServer::new(router).expect("build test server")
}

async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    for table in &[
        "cp_entitlements",
        "cp_tenant_bundle",
        "provisioning_outbox",
        "provisioning_requests",
        "tenants",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE tenant_id = $1"))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Caller with PLATFORM_TENANTS_CREATE gets 202 Accepted.
#[tokio::test]
async fn create_tenant_requires_permission_granted() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let token = mint_token(
        &keys.encoding,
        vec![security::permissions::PLATFORM_TENANTS_CREATE.into()],
    );
    let server = build_authed_server(pool.clone(), keys.verifier);
    let idem_key = format!("rbac-allowed-{}", Uuid::new_v4());

    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(token)
        .json(&serde_json::json!({
            "idempotency_key": idem_key,
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }))
        .await;

    resp.assert_status(StatusCode::ACCEPTED);

    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "pending");
    assert!(body["tenant_id"].is_string());

    let tenant_id: Uuid = body["tenant_id"].as_str().unwrap().parse().unwrap();
    cleanup(&pool, tenant_id).await;
}

/// Caller WITHOUT PLATFORM_TENANTS_CREATE gets 403 Forbidden.
#[tokio::test]
async fn create_tenant_requires_permission_denied() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    // Token has some other permission but not PLATFORM_TENANTS_CREATE
    let token = mint_token(
        &keys.encoding,
        vec!["ar.mutate".into(), "gl.post".into()],
    );
    let server = build_authed_server(pool.clone(), keys.verifier);

    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(token)
        .json(&serde_json::json!({
            "idempotency_key": format!("rbac-denied-{}", Uuid::new_v4()),
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }))
        .await;

    resp.assert_status(StatusCode::FORBIDDEN);

    let body: serde_json::Value = resp.json();
    assert_eq!(body["error"], "forbidden");
}

/// Caller with no JWT at all gets 401 Unauthorized.
#[tokio::test]
async fn create_tenant_requires_permission_no_jwt() {
    let pool = test_pool().await;
    let keys = make_test_keys();
    let server = build_authed_server(pool.clone(), keys.verifier);

    let resp = server
        .post("/api/control/tenants")
        .json(&serde_json::json!({
            "idempotency_key": format!("rbac-noauth-{}", Uuid::new_v4()),
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }))
        .await;

    resp.assert_status(StatusCode::UNAUTHORIZED);

    let body: serde_json::Value = resp.json();
    assert_eq!(body["error"], "unauthorized");
}

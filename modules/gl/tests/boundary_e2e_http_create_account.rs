//! Boundary E2E Test: POST /api/gl/accounts (Account Creation)
//!
//! Tests the account creation API endpoint through real HTTP requests.
//!
//! ## Prerequisites
//! - GL HTTP server at localhost:8090
//! - PostgreSQL at localhost:5438

use gl_rs::db::init_pool;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// JWT Auth Helpers
// ============================================================================

#[derive(Serialize)]
struct TestJwtClaims {
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

fn sign_test_jwt(tenant_id: &str, perms: Vec<String>) -> String {
    dotenvy::dotenv().ok();
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set (loaded from .env)");
    let encoding_key =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("Invalid JWT_PRIVATE_KEY_PEM");
    let now = chrono::Utc::now();
    let claims = TestJwtClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["operator".into()],
        perms,
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    let header = Header::new(Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &encoding_key).expect("Failed to sign test JWT")
}

fn authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token)
            .parse()
            .expect("valid header value"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("Failed to build authed client")
}

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string());
    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Response types for deserialization
// ============================================================================

#[derive(Debug, Deserialize)]
struct AccountResponse {
    id: String,
    tenant_id: String,
    code: String,
    name: String,
    account_type: String,
    normal_balance: String,
    is_active: bool,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

fn gl_url() -> String {
    std::env::var("GL_SERVICE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
#[ignore]
async fn test_create_account_returns_201() {
    let pool = setup_test_pool().await;
    let tenant_id = Uuid::new_v4().to_string();
    cleanup_test_data(&pool, &tenant_id).await;

    let token = sign_test_jwt(&tenant_id, vec!["gl.post".into()]);
    let client = authed_client(&token);

    let resp = client
        .post(format!("{}/api/gl/accounts", gl_url()))
        .json(&serde_json::json!({
            "code": "1000",
            "name": "Cash",
            "account_type": "Asset",
            "normal_balance": "Debit"
        }))
        .send()
        .await
        .expect("Failed to send request — is GL service running?");

    assert_eq!(resp.status(), 201, "Expected 201 Created");

    let body: AccountResponse = resp.json().await.expect("Failed to parse response");
    assert_eq!(body.code, "1000");
    assert_eq!(body.name, "Cash");
    assert_eq!(body.account_type, "Asset");
    assert_eq!(body.normal_balance, "Debit");
    assert_eq!(body.tenant_id, tenant_id);
    assert!(body.is_active);
    assert!(!body.id.is_empty());

    cleanup_test_data(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn test_duplicate_code_returns_409() {
    let pool = setup_test_pool().await;
    let tenant_id = Uuid::new_v4().to_string();
    cleanup_test_data(&pool, &tenant_id).await;

    let token = sign_test_jwt(&tenant_id, vec!["gl.post".into()]);
    let client = authed_client(&token);

    let body = serde_json::json!({
        "code": "2000",
        "name": "Accounts Payable",
        "account_type": "Liability",
        "normal_balance": "Credit"
    });

    // First create — should succeed
    let resp1 = client
        .post(format!("{}/api/gl/accounts", gl_url()))
        .json(&body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp1.status(), 201);

    // Second create with same code — should 409
    let resp2 = client
        .post(format!("{}/api/gl/accounts", gl_url()))
        .json(&body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp2.status(), 409, "Expected 409 Conflict for duplicate code");

    let err: ErrorResponse = resp2.json().await.expect("Failed to parse error response");
    assert!(err.error.contains("already exists"), "Error should mention 'already exists': {}", err.error);

    cleanup_test_data(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
#[ignore]
async fn test_missing_required_field_returns_422() {
    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_test_jwt(&tenant_id, vec!["gl.post".into()]);
    let client = authed_client(&token);

    // Missing account_type and normal_balance
    let resp = client
        .post(format!("{}/api/gl/accounts", gl_url()))
        .json(&serde_json::json!({
            "code": "3000",
            "name": "Test"
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert!(
        resp.status().is_client_error(),
        "Expected client error for missing fields, got {}",
        resp.status()
    );
}

#[tokio::test]
#[serial]
#[ignore]
async fn test_different_tenants_same_code() {
    let pool = setup_test_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();
    cleanup_test_data(&pool, &tenant_a).await;
    cleanup_test_data(&pool, &tenant_b).await;

    let body = serde_json::json!({
        "code": "4000",
        "name": "Revenue",
        "account_type": "Revenue",
        "normal_balance": "Credit"
    });

    // Tenant A
    let token_a = sign_test_jwt(&tenant_a, vec!["gl.post".into()]);
    let client_a = authed_client(&token_a);
    let resp_a = client_a
        .post(format!("{}/api/gl/accounts", gl_url()))
        .json(&body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp_a.status(), 201, "Tenant A should get 201");

    // Tenant B — same code, should also succeed
    let token_b = sign_test_jwt(&tenant_b, vec!["gl.post".into()]);
    let client_b = authed_client(&token_b);
    let resp_b = client_b
        .post(format!("{}/api/gl/accounts", gl_url()))
        .json(&body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(resp_b.status(), 201, "Tenant B should also get 201 — different tenant, same code");

    cleanup_test_data(&pool, &tenant_a).await;
    cleanup_test_data(&pool, &tenant_b).await;
}

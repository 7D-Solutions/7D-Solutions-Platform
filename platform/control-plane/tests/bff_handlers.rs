/// Integration tests for control-plane BFF handlers.
///
/// Tests the full HTTP request/response cycle for:
///   - POST /api/control/tenants (create tenant with provisioning)
///   - GET  /api/control/tenants/:tenant_id/retention (read retention)
///   - PUT  /api/control/tenants/:tenant_id/retention (upsert retention)
///   - POST /api/control/tenants/:tenant_id/tombstone (tombstone tenant data)
///   - GET  /healthz (liveness probe)
///   - GET  /api/ready (readiness probe)
///
/// All tests run against a real Postgres database. No mocks.
use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::io::Cursor;
use std::sync::{Arc, OnceLock};
use uuid::Uuid;
use zip::ZipArchive;

use control_plane::routes::build_router;
use control_plane::state::AppState;
use tenant_registry::routes::SummaryState;

// ============================================================================
// Test JWT helpers
// ============================================================================

struct TestKeys {
    encoding: EncodingKey,
    verifier: Arc<security::JwtVerifier>,
}

static TEST_KEYS: OnceLock<TestKeys> = OnceLock::new();

fn test_keys() -> &'static TestKeys {
    TEST_KEYS.get_or_init(|| {
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
    })
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

/// Mint a short-lived token carrying the given permissions.
fn mint_token(perms: Vec<String>) -> String {
    let keys = test_keys();
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
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding)
        .expect("sign token")
}

/// Token with the PLATFORM_TENANTS_CREATE permission.
fn create_tenant_token() -> String {
    mint_token(vec![security::permissions::PLATFORM_TENANTS_CREATE.into()])
}

// ============================================================================
// Helpers
// ============================================================================

async fn test_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });
    PgPool::connect(&url)
        .await
        .expect("connect to tenant-registry DB")
}

fn build_test_server(pool: PgPool) -> TestServer {
    let keys = test_keys();
    let app_state =
        Arc::new(AppState::new(pool.clone(), None).with_verifier(keys.verifier.clone()));
    let summary_state = Arc::new(SummaryState::new_local(pool));
    let router = build_router(app_state, summary_state);
    TestServer::new(router).expect("build test server")
}

/// Insert a tenant directly in the DB and return its tenant_id.
async fn seed_tenant(pool: &PgPool, status: &str, product_code: &str, plan_code: &str) -> Uuid {
    let tenant_id = Uuid::new_v4();
    let app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);
    sqlx::query(
        r#"INSERT INTO tenants
           (tenant_id, status, environment, module_schema_versions,
            product_code, plan_code, app_id, created_at, updated_at)
           VALUES ($1, $2, 'development', '{}'::jsonb, $3, $4, $5, NOW(), NOW())"#,
    )
    .bind(tenant_id)
    .bind(status)
    .bind(product_code)
    .bind(plan_code)
    .bind(&app_id)
    .execute(pool)
    .await
    .expect("insert tenant");
    tenant_id
}

/// Clean up a tenant and its related rows.
async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM cp_retention_policies WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_entitlements WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM cp_tenant_bundle WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM provisioning_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM provisioning_requests WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Healthz & Ready
// ============================================================================

#[tokio::test]
async fn healthz_returns_200() {
    let pool = test_pool().await;
    let server = build_test_server(pool);

    let resp = server.get("/healthz").await;
    resp.assert_status(StatusCode::OK);
}

#[tokio::test]
async fn ready_returns_200_with_db_check() {
    let pool = test_pool().await;
    let server = build_test_server(pool);

    let resp = server.get("/api/ready").await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["service_name"], "control-plane");
    assert!(body["checks"].is_array());
}

// ============================================================================
// POST /api/control/tenants — Create Tenant
// ============================================================================

#[tokio::test]
async fn create_tenant_returns_202_with_correct_fields() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let idem_key = format!("test-{}", Uuid::new_v4());

    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "idempotency_key": idem_key,
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }))
        .await;

    resp.assert_status(StatusCode::ACCEPTED);

    let body: Value = resp.json();
    assert_eq!(body["status"], "pending");
    assert_eq!(body["idempotency_key"], idem_key);
    assert_eq!(body["product_code"], "starter");
    assert_eq!(body["plan_code"], "monthly");
    assert_eq!(body["concurrent_user_limit"], 5); // default
    assert!(body["tenant_id"].is_string());
    assert!(body["app_id"].as_str().unwrap().starts_with("app-"));

    let tenant_id: Uuid = body["tenant_id"].as_str().unwrap().parse().unwrap();
    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn create_tenant_idempotency_replays_200() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let idem_key = format!("test-{}", Uuid::new_v4());

    let payload = json!({
        "idempotency_key": idem_key,
        "environment": "development",
        "product_code": "starter",
        "plan_code": "monthly"
    });

    // First call: 202
    let resp1 = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&payload)
        .await;
    resp1.assert_status(StatusCode::ACCEPTED);
    let body1: Value = resp1.json();
    let tenant_id: Uuid = body1["tenant_id"].as_str().unwrap().parse().unwrap();

    // Second call with same idem key: 200
    let resp2 = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&payload)
        .await;
    resp2.assert_status(StatusCode::OK);
    let body2: Value = resp2.json();

    assert_eq!(body1["tenant_id"], body2["tenant_id"]);
    assert_eq!(body2["product_code"], "starter");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn create_tenant_with_explicit_id_and_limit() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = Uuid::new_v4();
    let idem_key = format!("test-{}", Uuid::new_v4());

    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "tenant_id": tenant_id,
            "idempotency_key": idem_key,
            "environment": "production",
            "product_code": "enterprise",
            "plan_code": "annual",
            "concurrent_user_limit": 50
        }))
        .await;

    resp.assert_status(StatusCode::ACCEPTED);

    let body: Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert_eq!(body["concurrent_user_limit"], 50);
    assert_eq!(body["product_code"], "enterprise");
    assert_eq!(body["plan_code"], "annual");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn create_tenant_duplicate_id_returns_409() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "active", "starter", "monthly").await;
    let idem_key = format!("test-{}", Uuid::new_v4());

    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "tenant_id": tenant_id,
            "idempotency_key": idem_key,
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }))
        .await;

    resp.assert_status(StatusCode::CONFLICT);

    let body: Value = resp.json();
    assert!(body["error"].as_str().unwrap().contains("already exists"));

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn create_tenant_validation_errors_return_422() {
    let pool = test_pool().await;
    let server = build_test_server(pool);

    // Empty idempotency key
    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "idempotency_key": "",
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly"
        }))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // Empty product code
    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "idempotency_key": "key-1",
            "environment": "development",
            "product_code": "",
            "plan_code": "monthly"
        }))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // Empty plan code
    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "idempotency_key": "key-2",
            "environment": "development",
            "product_code": "starter",
            "plan_code": ""
        }))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // Invalid concurrent_user_limit
    let resp = server
        .post("/api/control/tenants")
        .authorization_bearer(create_tenant_token())
        .json(&json!({
            "idempotency_key": "key-3",
            "environment": "development",
            "product_code": "starter",
            "plan_code": "monthly",
            "concurrent_user_limit": 0
        }))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}

// ============================================================================
// GET /api/control/tenants/:tenant_id/retention
// ============================================================================

#[tokio::test]
async fn get_retention_returns_defaults_for_existing_tenant() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "active", "starter", "monthly").await;

    let resp = server
        .get(&format!("/api/control/tenants/{tenant_id}/retention"))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert_eq!(body["data_retention_days"], 2555);
    assert_eq!(body["export_format"], "jsonl");
    assert_eq!(body["auto_tombstone_days"], 30);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn get_retention_returns_404_for_missing_tenant() {
    let pool = test_pool().await;
    let server = build_test_server(pool);
    let missing_id = Uuid::new_v4();

    let resp = server
        .get(&format!("/api/control/tenants/{missing_id}/retention"))
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// PUT /api/control/tenants/:tenant_id/retention
// ============================================================================

#[tokio::test]
async fn set_retention_upserts_and_returns_config() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "active", "starter", "monthly").await;

    let resp = server
        .put(&format!("/api/control/tenants/{tenant_id}/retention"))
        .json(&json!({
            "data_retention_days": 365,
            "auto_tombstone_days": 7
        }))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert_eq!(body["data_retention_days"], 365);
    assert_eq!(body["auto_tombstone_days"], 7);
    assert_eq!(body["export_format"], "jsonl");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn set_retention_rejects_invalid_values() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "active", "starter", "monthly").await;

    // data_retention_days <= 0
    let resp = server
        .put(&format!("/api/control/tenants/{tenant_id}/retention"))
        .json(&json!({"data_retention_days": 0}))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // auto_tombstone_days < 0
    let resp = server
        .put(&format!("/api/control/tenants/{tenant_id}/retention"))
        .json(&json!({"auto_tombstone_days": -1}))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn set_retention_returns_404_for_missing_tenant() {
    let pool = test_pool().await;
    let server = build_test_server(pool);
    let missing_id = Uuid::new_v4();

    let resp = server
        .put(&format!("/api/control/tenants/{missing_id}/retention"))
        .json(&json!({"data_retention_days": 365}))
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

// ============================================================================
// POST /api/control/tenants/:tenant_id/tombstone
// ============================================================================

#[tokio::test]
async fn tombstone_requires_deleted_status() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "active", "starter", "monthly").await;

    let resp = server
        .post(&format!("/api/control/tenants/{tenant_id}/tombstone"))
        .await;
    resp.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let body: Value = resp.json();
    assert!(body["error"].as_str().unwrap().contains("deleted"));

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tombstone_succeeds_for_deleted_tenant() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "deleted", "starter", "monthly").await;

    let resp = server
        .post(&format!("/api/control/tenants/{tenant_id}/tombstone"))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert!(body["data_tombstoned_at"].is_string());
    assert!(body["audit_note"].as_str().unwrap().contains("tombstone"));

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tombstone_is_idempotent() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "deleted", "starter", "monthly").await;

    // First tombstone
    let resp1 = server
        .post(&format!("/api/control/tenants/{tenant_id}/tombstone"))
        .await;
    resp1.assert_status(StatusCode::OK);
    let body1: Value = resp1.json();

    // Second tombstone — idempotent replay
    let resp2 = server
        .post(&format!("/api/control/tenants/{tenant_id}/tombstone"))
        .await;
    resp2.assert_status(StatusCode::OK);
    let body2: Value = resp2.json();

    assert_eq!(body1["data_tombstoned_at"], body2["data_tombstoned_at"]);
    assert!(body2["audit_note"].as_str().unwrap().contains("idempotent"));

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn tombstone_returns_404_for_missing_tenant() {
    let pool = test_pool().await;
    let server = build_test_server(pool);
    let missing_id = Uuid::new_v4();

    let resp = server
        .post(&format!("/api/control/tenants/{missing_id}/tombstone"))
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gdpr_erasure_alias_matches_tombstone_behavior() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "deleted", "starter", "monthly").await;

    let resp = server
        .post(&format!("/api/control/tenants/{tenant_id}/gdpr-erasure"))
        .await;
    resp.assert_status(StatusCode::OK);

    let body: Value = resp.json();
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert!(body["data_tombstoned_at"].is_string());
    assert!(body["audit_note"].as_str().unwrap().contains("tombstone"));

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn export_returns_zip_bundle_and_updates_export_ready_at() {
    let pool = test_pool().await;
    let server = build_test_server(pool.clone());
    let tenant_id = seed_tenant(&pool, "deleted", "starter", "monthly").await;

    sqlx::query(
        r#"INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit, effective_at, updated_at)
           VALUES ($1, $2, $3, NOW(), NOW())
           ON CONFLICT (tenant_id) DO UPDATE
               SET plan_code = EXCLUDED.plan_code,
                   concurrent_user_limit = EXCLUDED.concurrent_user_limit,
                   updated_at = EXCLUDED.updated_at"#,
    )
    .bind(tenant_id)
    .bind("monthly")
    .bind(7_i32)
    .execute(&pool)
    .await
    .expect("seed entitlements");

    let resp = server
        .post(&format!("/api/control/tenants/{tenant_id}/export"))
        .await;
    resp.assert_status(StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/zip")
    );

    let bytes = resp.into_bytes();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip archive");
    let mut names = Vec::new();
    for i in 0..archive.len() {
        names.push(archive.by_index(i).unwrap().name().to_string());
    }
    assert_eq!(
        names,
        vec![
            "tenant.jsonl".to_string(),
            "retention_policy.jsonl".to_string(),
            "entitlements.jsonl".to_string(),
            "provisioning_requests.jsonl".to_string(),
            "manifest.json".to_string(),
        ]
    );

    let retention: Option<(chrono::DateTime<Utc>,)> =
        sqlx::query_as("SELECT export_ready_at FROM cp_retention_policies WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(&pool)
            .await
            .expect("query export_ready_at");
    assert!(retention.and_then(|row| Some(row.0)).is_some());

    cleanup(&pool, tenant_id).await;
}

//! Integration tests for QBO webhook verifier token admin API.
//!
//! Verified against real Postgres (no mocks, no stubs).
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test qbo_webhook_token_api_test

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    routing::{get, post},
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{http::qbo_settings, metrics::IntegrationsMetrics, AppState};
use security::{claims::ActorType, VerifiedClaims};
use serde_json::Value;
use serial_test::serial;
use tower::ServiceExt;
use uuid::Uuid;

// ── AES-256-GCM test key ──────────────────────────────────────────────────────

const TEST_KEY: [u8; 32] = [0x42u8; 32];

// ── DB pool ───────────────────────────────────────────────────────────────────

fn test_db_url() -> String {
    dotenvy::dotenv().ok();
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    })
}

async fn test_pool() -> sqlx::PgPool {
    let pool = sqlx::PgPool::connect(&test_db_url())
        .await
        .expect("connect to integrations test db");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

// ── Claims helpers ────────────────────────────────────────────────────────────

fn test_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["integrations.mutate".into(), "integrations.read".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

// ── Router builders ───────────────────────────────────────────────────────────

fn build_app(pool: sqlx::PgPool, tenant_id: Uuid) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: TEST_KEY,
    });
    Router::new()
        .route(
            "/api/integrations/qbo/webhook-token",
            post(qbo_settings::set_webhook_token),
        )
        .route(
            "/api/integrations/qbo/webhook-token/status",
            get(qbo_settings::webhook_token_status),
        )
        .with_state(state)
        .layer(Extension(test_claims(tenant_id)))
}

fn build_full_app_no_auth(pool: sqlx::PgPool) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: TEST_KEY,
    });
    integrations_rs::http::router(state)
}

// ── DB helpers ────────────────────────────────────────────────────────────────

async fn cleanup_secrets(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_qbo_webhook_secrets WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn cleanup_connection(pool: &sqlx::PgPool, realm_id: &str) {
    sqlx::query(
        "DELETE FROM integrations_oauth_connections WHERE provider = 'quickbooks' AND realm_id = $1",
    )
    .bind(realm_id)
    .execute(pool)
    .await
    .ok();
}

async fn seed_oauth_connection(pool: &sqlx::PgPool, app_id: &str, realm_id: &str) {
    cleanup_connection(pool, realm_id).await;

    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections
            (app_id, provider, realm_id,
             access_token, refresh_token,
             access_token_expires_at, refresh_token_expires_at,
             scopes_granted, connection_status)
        VALUES
            ($1, 'quickbooks', $2,
             pgp_sym_encrypt('dummy-access', 'test-key'),
             pgp_sym_encrypt('dummy-refresh', 'test-key'),
             NOW() + INTERVAL '1 hour',
             NOW() + INTERVAL '90 days',
             'com.intuit.quickbooks.accounting',
             'connected')
        ON CONFLICT (app_id, provider) DO UPDATE
            SET realm_id = EXCLUDED.realm_id,
                connection_status = 'connected',
                updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .execute(pool)
    .await
    .expect("seed oauth connection");
}

async fn response_body(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// POST with valid realm_id+token returns 204; GET status then returns configured=true.
#[tokio::test]
#[serial]
async fn post_valid_token_returns_204_and_status_shows_configured() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    let realm_id = format!("realm-{}", Uuid::new_v4());

    cleanup_secrets(&pool, &app_id).await;
    seed_oauth_connection(&pool, &app_id, &realm_id).await;

    // POST — set token
    let post_req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/qbo/webhook-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "realm_id": &realm_id, "token": "my-verifier-token" }).to_string(),
        ))
        .unwrap();

    let app = build_app(pool.clone(), tenant_id);
    let resp = app.oneshot(post_req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "POST must return 204"
    );

    // GET — check status
    let get_req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/qbo/webhook-token/status?realm_id={}",
            realm_id
        ))
        .body(Body::empty())
        .unwrap();

    let app2 = build_app(pool.clone(), tenant_id);
    let resp2 = app2.oneshot(get_req).await.expect("oneshot");
    assert_eq!(resp2.status(), StatusCode::OK, "GET must return 200");

    let body = response_body(resp2).await;
    assert_eq!(
        body["configured"], true,
        "configured must be true after POST"
    );
    assert!(
        body["last_set_at"].is_string(),
        "last_set_at must be a non-null string after POST"
    );

    cleanup_secrets(&pool, &app_id).await;
    cleanup_connection(&pool, &realm_id).await;
}

/// POST with empty realm_id returns 400.
#[tokio::test]
#[serial]
async fn post_empty_realm_id_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();

    let app = build_app(pool, tenant_id);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/qbo/webhook-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "realm_id": "", "token": "some-token" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "empty realm_id must return 400"
    );
}

/// POST with empty token returns 400.
#[tokio::test]
#[serial]
async fn post_empty_token_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();

    let app = build_app(pool, tenant_id);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/qbo/webhook-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "realm_id": "some-realm", "token": "" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "empty token must return 400"
    );
}

/// POST with a realm_id that has no connected OAuth account returns 404.
#[tokio::test]
#[serial]
async fn post_realm_id_with_no_connection_returns_404() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let unconnected_realm = format!("no-connection-{}", Uuid::new_v4());

    let app = build_app(pool, tenant_id);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/qbo/webhook-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "realm_id": &unconnected_realm, "token": "some-token" })
                .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "realm_id with no connection must return 404"
    );
}

/// GET with missing realm_id param returns 400.
#[tokio::test]
#[serial]
async fn get_missing_realm_id_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();

    let app = build_app(pool, tenant_id);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/qbo/webhook-token/status")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "missing realm_id param must return 400"
    );
}

/// GET with realm_id='' (empty string) returns 400.
#[tokio::test]
#[serial]
async fn get_empty_realm_id_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();

    let app = build_app(pool, tenant_id);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/qbo/webhook-token/status?realm_id=")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "empty realm_id param must return 400"
    );
}

/// GET before any POST for a given realm returns configured=false with null last_set_at.
#[tokio::test]
#[serial]
async fn get_before_post_returns_not_configured() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    let realm_id = format!("unconfigured-{}", Uuid::new_v4());

    cleanup_secrets(&pool, &app_id).await;

    let app = build_app(pool, tenant_id);
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/qbo/webhook-token/status?realm_id={}",
            realm_id
        ))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK, "GET must return 200");

    let body = response_body(resp).await;
    assert_eq!(
        body["configured"], false,
        "configured must be false before any POST"
    );
    assert!(
        body["last_set_at"].is_null(),
        "last_set_at must be null when not configured"
    );
}

/// POST without a JWT returns 401 (RequirePermissionsLayer enforces auth).
#[tokio::test]
#[serial]
async fn post_without_auth_returns_401() {
    let pool = test_pool().await;
    let app = build_full_app_no_auth(pool);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/qbo/webhook-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "realm_id": "any-realm", "token": "any-token" }).to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST without JWT must return 401 — RequirePermissionsLayer must be registered"
    );
}

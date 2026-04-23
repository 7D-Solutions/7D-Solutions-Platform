//! Integration tests for carrier credentials admin API (bd-e3vwo).
//!
//! All tests use real Postgres. No mocks, no stubs.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs -- carrier_credentials

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{
    http::carrier_credentials::{carrier_credentials_status, set_carrier_credentials},
    metrics::IntegrationsMetrics,
    AppState,
};
use security::{claims::ActorType, VerifiedClaims};
use serde_json::Value;
use serial_test::serial;
use tower::ServiceExt;
use uuid::Uuid;

// ── Constants ─────────────────────────────────────────────────────────────────

const TEST_KEY: [u8; 32] = [0x42u8; 32];

// ── DB helpers ────────────────────────────────────────────────────────────────

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

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_carrier_credentials WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn response_body(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ── Claims helpers ────────────────────────────────────────────────────────────

fn mutate_claims(tenant_id: Uuid) -> VerifiedClaims {
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

fn read_only_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["viewer".into()],
        perms: vec!["integrations.read".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

// ── Router builders ───────────────────────────────────────────────────────────

fn build_app_with_claims(pool: sqlx::PgPool, claims: VerifiedClaims) -> Router {
    use axum::routing::{get, post};
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: TEST_KEY,
    });
    Router::new()
        .route(
            "/api/integrations/carriers/{carrier_type}/credentials",
            post(set_carrier_credentials),
        )
        .route(
            "/api/integrations/carriers/{carrier_type}/credentials/status",
            get(carrier_credentials_status),
        )
        .with_state(state)
        .layer(Extension(claims))
}

fn build_full_router_no_auth(pool: sqlx::PgPool) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: TEST_KEY,
    });
    integrations_rs::http::router(state)
}

fn build_full_router_with_claims(pool: sqlx::PgPool, claims: VerifiedClaims) -> Router {
    build_full_router_no_auth(pool).layer(Extension(claims))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// POST UPS credentials returns 204; subsequent GET includes last_set_at.
#[tokio::test]
#[serial]
async fn carrier_credentials_ups_post_returns_204() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    let app = build_app_with_claims(pool, mutate_claims(tenant_id));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/ups/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "client_id": "ups-cid",
                "client_secret": "ups-secret",
                "account_number": "12345678"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "UPS POST must return 204");
}

/// POST UPS then GET status — summary contains last 4 of account_number.
#[tokio::test]
#[serial]
async fn carrier_credentials_ups_get_configured() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    let app = build_app_with_claims(pool.clone(), mutate_claims(tenant_id));

    let post_req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/ups/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "client_id": "ups-cid",
                "client_secret": "ups-secret",
                "account_number": "ACCT9876"
            })
            .to_string(),
        ))
        .unwrap();
    app.oneshot(post_req).await.expect("POST");

    let app2 = build_app_with_claims(pool, mutate_claims(tenant_id));
    let get_req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/carriers/ups/credentials/status")
        .body(Body::empty())
        .unwrap();
    let resp = app2.oneshot(get_req).await.expect("GET");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["configured"], true);
    assert!(body["last_set_at"].as_str().is_some(), "last_set_at must be present");
    let summary = body["summary"].as_str().expect("summary must be present");
    assert!(
        summary.contains("9876"),
        "summary must contain last 4 of account_number, got: {}",
        summary
    );
    // RFC-3339 date-only in the summary
    assert!(summary.contains("set 20"), "summary must contain set date, got: {}", summary);
}

/// POST FedEx credentials with client_id/client_secret/account_number; GET returns configured.
#[tokio::test]
#[serial]
async fn carrier_credentials_fedex_post_and_status() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    let app = build_app_with_claims(pool.clone(), mutate_claims(tenant_id));
    let post_req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/fedex/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "client_id": "fdx-cid",
                "client_secret": "fdx-secret",
                "account_number": "FDX00123"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(post_req).await.expect("POST");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app2 = build_app_with_claims(pool, mutate_claims(tenant_id));
    let get_req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/carriers/fedex/credentials/status")
        .body(Body::empty())
        .unwrap();
    let resp = app2.oneshot(get_req).await.expect("GET");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["configured"], true);
    let summary = body["summary"].as_str().expect("summary");
    assert!(summary.contains("0123"), "FedEx summary must contain last 4 of account_number");
}

/// POST USPS with user_id only (password absent); GET returns User ID summary.
#[tokio::test]
#[serial]
async fn carrier_credentials_usps_post_and_status() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    let app = build_app_with_claims(pool.clone(), mutate_claims(tenant_id));
    let post_req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/usps/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "user_id": "MYUSER9999" }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(post_req).await.expect("POST");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app2 = build_app_with_claims(pool, mutate_claims(tenant_id));
    let get_req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/carriers/usps/credentials/status")
        .body(Body::empty())
        .unwrap();
    let resp = app2.oneshot(get_req).await.expect("GET");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["configured"], true);
    let summary = body["summary"].as_str().expect("summary");
    assert!(
        summary.starts_with("User ID ..."),
        "USPS summary must start with 'User ID ...', got: {}",
        summary
    );
    assert!(summary.contains("9999"), "USPS summary must contain last 4 of user_id");
}

/// Unknown carrier type returns 400.
#[tokio::test]
#[serial]
async fn carrier_credentials_unknown_type_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app_with_claims(pool, mutate_claims(tenant_id));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/dhl/credentials")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({ "client_id": "x" }).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// UPS POST with missing required field returns 400.
#[tokio::test]
#[serial]
async fn carrier_credentials_ups_missing_field_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app_with_claims(pool, mutate_claims(tenant_id));

    // Missing account_number
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/ups/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "client_id": "ups-cid",
                "client_secret": "ups-secret"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// USPS POST with missing user_id returns 400.
#[tokio::test]
#[serial]
async fn carrier_credentials_usps_missing_user_id_returns_400() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app_with_claims(pool, mutate_claims(tenant_id));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/usps/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "password": "some-pass" }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST without auth returns 401 (RequirePermissionsLayer enforces auth).
#[tokio::test]
#[serial]
async fn carrier_credentials_no_auth_returns_401() {
    let pool = test_pool().await;
    let app = build_full_router_no_auth(pool);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/ups/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "client_id": "x",
                "client_secret": "y",
                "account_number": "z"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST without JWT must return 401"
    );
}

/// JWT without INTEGRATIONS_MUTATE claim returns 403 on POST.
#[tokio::test]
#[serial]
async fn carrier_credentials_wrong_permission_returns_403() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    // Inject read-only claims (no integrations.mutate)
    let app = build_full_router_with_claims(pool, read_only_claims(tenant_id));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/carriers/ups/credentials")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "client_id": "x",
                "client_secret": "y",
                "account_number": "z"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST with read-only JWT must return 403"
    );
}

/// GET status before any POST returns configured=false.
#[tokio::test]
#[serial]
async fn carrier_credentials_get_before_post_returns_unconfigured() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    let app = build_app_with_claims(pool, mutate_claims(tenant_id));
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/carriers/ups/credentials/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["configured"], false);
    assert!(body["last_set_at"].is_null());
    assert!(body["summary"].is_null());
}

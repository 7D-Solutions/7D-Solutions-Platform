//! Tests for UPS and FedEx OAuth provider support (bd-8v2c8).
//!
//! All tests use real Postgres and the in-process axum router — no mocks.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs -- oauth_carrier

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{metrics::IntegrationsMetrics, AppState};
use security::{claims::ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

// ── DB helpers ────────────────────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect to integrations test db")
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

// ── Router builder ────────────────────────────────────────────────────────────

fn build_app(pool: sqlx::PgPool, claims: VerifiedClaims) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    integrations_rs::http::router(state).layer(Extension(claims))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// GET /api/integrations/oauth/connect/ups → 307 with Location pointing to onlinetools.ups.com
#[tokio::test]
#[serial]
async fn oauth_carrier_ups_connect_redirects() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app(pool, test_claims(tenant_id));

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/oauth/connect/ups")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "UPS connect must return 307"
    );
    let location = resp
        .headers()
        .get("location")
        .expect("location header must be present")
        .to_str()
        .expect("location must be valid UTF-8");
    assert!(
        location.contains("onlinetools.ups.com"),
        "Location must point to UPS auth server, got: {}",
        location
    );
}

/// GET /api/integrations/oauth/connect/fedex → 307 with Location pointing to apis.fedex.com
#[tokio::test]
#[serial]
async fn oauth_carrier_fedex_connect_redirects() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app(pool, test_claims(tenant_id));

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/oauth/connect/fedex")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "FedEx connect must return 307"
    );
    let location = resp
        .headers()
        .get("location")
        .expect("location header must be present")
        .to_str()
        .expect("location must be valid UTF-8");
    assert!(
        location.contains("apis.fedex.com"),
        "Location must point to FedEx auth server, got: {}",
        location
    );
}

/// GET /api/integrations/oauth/connect/dhl → 422 (unsupported provider)
#[tokio::test]
#[serial]
async fn oauth_carrier_unknown_provider_returns_422() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app(pool, test_claims(tenant_id));

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/oauth/connect/dhl")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Unknown provider must return 422"
    );
}

/// GET /api/integrations/oauth/callback/quickbooks without realmId → 400 or 422
/// Verifies backward compatibility: quickbooks still requires realmId after making it optional for UPS/FedEx.
#[tokio::test]
#[serial]
async fn oauth_carrier_quickbooks_realm_id_still_required() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app = build_app(pool, test_claims(tenant_id));

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/oauth/callback/quickbooks?code=dummycode&state=some-tenant-id")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");

    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "quickbooks callback without realmId must return 400 or 422, got {}",
        status
    );
}

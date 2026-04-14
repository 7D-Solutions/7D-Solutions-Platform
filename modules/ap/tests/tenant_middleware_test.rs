//! HTTP-layer tenant isolation tests for AP (bd-u0qut).
//!
//! Proves that the auth middleware blocks unauthorized requests and that
//! per-tenant scoping prevents cross-tenant data visibility through the HTTP API.
//!
//! ## Strategy
//!
//! An `inject_claims` middleware reads a `X-Tenant-Id: <uuid>` header and
//! inserts `VerifiedClaims` into request extensions — the same path the JWT
//! middleware follows in production.  Handlers use `extract_tenant()` which
//! reads those claims; without them it returns 401.
//!
//! Three assertions:
//! 1. No `X-Tenant-Id` header → 401
//! 2. Tenant A's UUID in header → sees only Tenant A's vendors
//! 3. Tenant B's UUID in header → does not see Tenant A's vendors
//!
//! ## Prerequisites
//!
//! PostgreSQL reachable at `DATABASE_URL` (default: localhost:5443).

use ap::{
    domain::vendors::{service::create_vendor, CreateVendorRequest},
    metrics::ApMetrics,
    AppState,
};
use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use http_body_util::BodyExt;
use security::{claims::ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ── DB setup ─────────────────────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("AP test DB connect");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("AP migrations");
    pool
}

// ── Claims injection middleware ───────────────────────────────────────────────

/// Reads `X-Tenant-Id: <uuid>` and injects `VerifiedClaims` into extensions.
/// Without the header the request passes through without claims — `extract_tenant`
/// will then return 401.
async fn inject_claims(req: Request, next: Next) -> Response {
    let tenant_id = req
        .headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok());

    match tenant_id {
        Some(tid) => {
            let claims = VerifiedClaims {
                user_id: Uuid::new_v4(),
                tenant_id: tid,
                app_id: None,
                roles: vec!["admin".to_string()],
                perms: vec!["ap.read".to_string(), "ap.mutate".to_string()],
                actor_type: ActorType::User,
                issued_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                token_id: Uuid::new_v4(),
                version: "1".to_string(),
            };
            let mut req = req;
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        None => next.run(req).await,
    }
}

// ── Test router ───────────────────────────────────────────────────────────────

/// Build the vendor-list route without permission enforcement.
/// The `inject_claims` middleware still exercises `extract_tenant`'s 401 guard.
fn build_test_app(pool: sqlx::PgPool) -> Router {
    let metrics = Arc::new(ApMetrics::new().expect("metrics init"));
    let state = Arc::new(AppState { pool, metrics, gl_pool: None });
    Router::new()
        .route("/api/ap/vendors", get(ap::http::vendors::list_vendors))
        .layer(middleware::from_fn(inject_claims))
        .with_state(state)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn vendor_req(name: &str) -> CreateVendorRequest {
    CreateVendorRequest {
        name: name.to_string(),
        tax_id: None,
        currency: "USD".to_string(),
        payment_terms_days: 30,
        payment_method: Some("ach".to_string()),
        remittance_email: None,
        party_id: None,
    }
}

async fn body_json(resp: Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// No `X-Tenant-Id` header → middleware passes request through without claims →
/// `extract_tenant` returns 401.
#[tokio::test]
#[serial]
async fn no_header_returns_401() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ap/vendors")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "Missing auth must return 401"
    );
}

/// A tenant with claims sees only their own vendors (not another tenant's).
#[tokio::test]
#[serial]
async fn tenant_sees_own_vendors() {
    let pool = setup_db().await;
    let tid_a = Uuid::new_v4();

    // Seed one vendor for tenant A using their UUID as tenant_id.
    create_vendor(&pool, &tid_a.to_string(), &vendor_req("MW-AP-VendorA"), corr())
        .await
        .expect("seed vendor A");

    let app = build_test_app(pool);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ap/vendors")
                .header("x-tenant-id", tid_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let items = json["data"].as_array().expect("data array");
    assert!(
        items.iter().any(|v| v["name"] == "MW-AP-VendorA"),
        "Tenant A must see their own vendor"
    );
}

/// Tenant B's claims return an empty vendor list — they cannot see Tenant A's data.
#[tokio::test]
#[serial]
async fn cross_tenant_vendor_list_is_empty() {
    let pool = setup_db().await;
    let tid_a = Uuid::new_v4();
    let tid_b = Uuid::new_v4();

    // Seed a vendor for Tenant A only.
    create_vendor(
        &pool,
        &tid_a.to_string(),
        &vendor_req("MW-AP-CrossTenantVendor"),
        corr(),
    )
    .await
    .expect("seed vendor A");

    // Request with Tenant B's UUID — must not see Tenant A's data.
    let app = build_test_app(pool);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ap/vendors")
                .header("x-tenant-id", tid_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let items = json["data"].as_array().expect("data array");
    assert!(
        !items.iter().any(|v| v["name"] == "MW-AP-CrossTenantVendor"),
        "Tenant B must not see Tenant A's vendor through the HTTP API"
    );
}

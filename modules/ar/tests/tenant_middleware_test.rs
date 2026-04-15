//! HTTP-layer tenant isolation tests for AR (bd-u0qut).
//!
//! Proves that the auth middleware blocks unauthorized requests and that
//! per-tenant scoping prevents cross-tenant data visibility through the HTTP API.
//!
//! ## Strategy
//!
//! An `inject_claims` middleware reads a `X-Tenant-Id: <uuid>` header and
//! inserts `VerifiedClaims` into request extensions.  AR handlers call
//! `extract_tenant()` (from `ar_rs::http::tenant`) which reads those claims;
//! without them it returns 401.
//!
//! Three assertions:
//! 1. No `X-Tenant-Id` header → 401
//! 2. Tenant A's UUID in header → sees Tenant A's customers (not B's)
//! 3. Tenant B's UUID in header → does not see Tenant A's customers
//!
//! ## Prerequisites
//!
//! PostgreSQL reachable at `DATABASE_URL_AR`.

use ar_rs::http::ar_router_permissive;
use axum::{
    body::Body,
    extract::Request,
    middleware::{self, Next},
    response::Response,
    Router,
};
use http_body_util::BodyExt;
use security::{claims::ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

// ── DB setup ─────────────────────────────────────────────────────────────────

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL_AR")
        .expect("DATABASE_URL_AR must be set for AR integration tests");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .idle_timeout(Some(std::time::Duration::from_secs(30)))
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("AR test DB connect");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("AR migrations");
    pool
}

// ── Claims injection middleware ───────────────────────────────────────────────

/// Reads `X-Tenant-Id: <uuid>` and injects `VerifiedClaims` into extensions.
/// Without the header the request passes through without claims — `extract_tenant`
/// returns 401.
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
                perms: vec!["ar.read".to_string(), "ar.mutate".to_string()],
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

/// Wrap `ar_router_permissive` with `inject_claims`.
/// Pass tenant via `X-Tenant-Id: <uuid>` header.
fn build_test_app(pool: PgPool) -> Router {
    ar_router_permissive(pool).layer(middleware::from_fn(inject_claims))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn body_json(resp: Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// Seed a customer directly in the DB for a given `app_id` (tenant UUID string).
async fn seed_customer(pool: &PgPool, app_id: &str, name: &str) {
    let email = format!("mw-ar-{}@test.example", Uuid::new_v4());
    let external_id = format!("ext-{}", Uuid::new_v4());
    sqlx::query(
        r#"INSERT INTO ar_customers
            (app_id, email, external_customer_id, status, name,
             default_payment_method_id, payment_method_type,
             retry_attempt_count, created_at, updated_at)
           VALUES ($1, $2, $3, 'active', $4, 'pm_test', 'card', 0, NOW(), NOW())"#,
    )
    .bind(app_id)
    .bind(&email)
    .bind(&external_id)
    .bind(name)
    .execute(pool)
    .await
    .expect("seed customer");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// No `X-Tenant-Id` header → middleware passes request without claims →
/// AR's `extract_tenant` returns 401.
#[tokio::test]
#[serial]
async fn no_header_returns_401() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        axum::http::StatusCode::UNAUTHORIZED,
        "Missing auth must return 401"
    );
}

/// A tenant with claims sees only their own customers.
#[tokio::test]
#[serial]
async fn tenant_sees_own_customers() {
    let pool = setup_db().await;
    let tid_a = Uuid::new_v4();

    seed_customer(&pool, &tid_a.to_string(), "MW-AR-CustomerA").await;

    let app = build_test_app(pool);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers")
                .header("x-tenant-id", tid_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let json = body_json(resp).await;
    let items = json["data"].as_array().expect("data array");
    assert!(
        items.iter().any(|v| v["name"] == "MW-AR-CustomerA"),
        "Tenant A must see their own customer"
    );
}

/// Tenant B's claims return an empty list — they cannot see Tenant A's customers.
#[tokio::test]
#[serial]
async fn cross_tenant_customer_list_is_empty() {
    let pool = setup_db().await;
    let tid_a = Uuid::new_v4();
    let tid_b = Uuid::new_v4();

    // Seed a customer for Tenant A only.
    seed_customer(&pool, &tid_a.to_string(), "MW-AR-CrossTenantCustomer").await;

    // Request with Tenant B's UUID.
    let app = build_test_app(pool);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers")
                .header("x-tenant-id", tid_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let json = body_json(resp).await;
    let items = json["data"].as_array().expect("data array");
    assert!(
        !items
            .iter()
            .any(|v| v["name"] == "MW-AR-CrossTenantCustomer"),
        "Tenant B must not see Tenant A's customers through the HTTP API"
    );
}

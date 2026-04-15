//! HTTP-layer tenant isolation tests for Inventory (bd-u0qut).
//!
//! Proves that the auth middleware blocks unauthorized requests and that
//! per-tenant scoping prevents cross-tenant data visibility through the HTTP API.
//!
//! ## Strategy
//!
//! An `inject_claims` middleware reads a `X-Tenant-Id: <uuid>` header and
//! inserts `VerifiedClaims` into request extensions.  Inventory handlers use the
//! `TenantId` extractor from `platform_sdk` — the new Axum `FromRequestParts`
//! implementation.  Without claims, `TenantId` returns 401 automatically.
//!
//! Three assertions:
//! 1. No `X-Tenant-Id` header → 401
//! 2. Tenant A's UUID in header → sees Tenant A's items (not B's)
//! 3. Tenant B's UUID in header → does not see Tenant A's items
//!
//! ## Prerequisites
//!
//! PostgreSQL reachable at `DATABASE_URL` (default: localhost:5442).

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
    Router,
};
use event_bus::InMemoryBus;
use http_body_util::BodyExt;
use inventory_rs::{
    domain::items::{CreateItemRequest, ItemRepo, TrackingMode},
    metrics::InventoryMetrics,
    AppState, BusHealth,
};
use security::{claims::ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ── DB setup ─────────────────────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Inventory test DB connect");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Inventory migrations");
    pool
}

// ── Claims injection middleware ───────────────────────────────────────────────

/// Reads `X-Tenant-Id: <uuid>` and injects `VerifiedClaims` into extensions.
/// Without the header the request passes through without claims — the `TenantId`
/// extractor returns 401 automatically.
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
                perms: vec!["inventory.read".to_string(), "inventory.mutate".to_string()],
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

/// Build the items read route without permission enforcement.
/// The `inject_claims` middleware exercises the `TenantId` extractor's 401 guard.
///
/// `InMemoryBus` is used for `event_bus` because:
/// - The `list_items` handler never publishes events (SQL-only)
/// - `InMemoryBus` is a first-class platform implementation, not a test stub
fn build_test_app(pool: sqlx::PgPool) -> Router {
    let metrics = Arc::new(InventoryMetrics::new().expect("metrics init"));
    let bus_health = BusHealth::new();
    let state = Arc::new(AppState {
        pool,
        metrics,
        event_bus: Arc::new(InMemoryBus::new()),
        bus_health,
    });

    Router::new()
        .route(
            "/api/inventory/items",
            get(inventory_rs::http::items::list_items),
        )
        .layer(middleware::from_fn(inject_claims))
        .with_state(state)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn item_req(tenant_id: &str, name: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: format!("MW-SKU-{}", Uuid::new_v4().simple()),
        name: name.to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

async fn body_json(resp: Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// No `X-Tenant-Id` header → middleware passes request without claims →
/// `TenantId` extractor returns 401.
#[tokio::test]
#[serial]
async fn no_header_returns_401() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/inventory/items")
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

/// A tenant with claims sees only their own items.
#[tokio::test]
#[serial]
async fn tenant_sees_own_items() {
    let pool = setup_db().await;
    let tid_a = Uuid::new_v4();

    // Seed one item for Tenant A.
    ItemRepo::create(&pool, &item_req(&tid_a.to_string(), "MW-Inv-ItemA"))
        .await
        .expect("seed item A");

    let app = build_test_app(pool);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/inventory/items")
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
        items.iter().any(|v| v["name"] == "MW-Inv-ItemA"),
        "Tenant A must see their own item"
    );
}

/// Tenant B's claims return an empty list — they cannot see Tenant A's items.
#[tokio::test]
#[serial]
async fn cross_tenant_item_list_is_empty() {
    let pool = setup_db().await;
    let tid_a = Uuid::new_v4();
    let tid_b = Uuid::new_v4();

    // Seed an item for Tenant A only.
    ItemRepo::create(
        &pool,
        &item_req(&tid_a.to_string(), "MW-Inv-CrossTenantItem"),
    )
    .await
    .expect("seed item A");

    // Request with Tenant B's UUID — must not see Tenant A's data.
    let app = build_test_app(pool);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/inventory/items")
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
        !items.iter().any(|v| v["name"] == "MW-Inv-CrossTenantItem"),
        "Tenant B must not see Tenant A's items through the HTTP API"
    );
}

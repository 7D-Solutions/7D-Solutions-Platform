//! HTTP integration tests for GET /api/integrations/sync/authority.
//!
//! Verifies:
//!   - empty array returned when tenant has no authority rows
//!   - rows for caller's tenant are returned with correct fields
//!   - rows belonging to another tenant are NOT returned (cross-tenant isolation)
//!
//! Requires a real Postgres instance. No mocks, no stubs.
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs --test sync_authority_endpoint_test

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{
    http::sync::get_authority_state,
    metrics::IntegrationsMetrics,
    AppState,
};
use security::{claims::ActorType, VerifiedClaims};
use serde_json::Value;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;

// ── DB pool ───────────────────────────────────────────────────────────────────

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn test_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["integrations.sync.read".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn build_app(pool: sqlx::PgPool, tenant_id: Uuid) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    Router::new()
        .route("/api/integrations/sync/authority", get(get_authority_state))
        .with_state(state)
        .layer(Extension(test_claims(tenant_id)))
}

async fn seed_authority(pool: &sqlx::PgPool, app_id: &str, provider: &str, entity_type: &str, side: &str) {
    sqlx::query(
        r#"
        INSERT INTO integrations_sync_authority
            (app_id, provider, entity_type, authoritative_side)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (app_id, provider, entity_type)
        DO UPDATE SET authoritative_side = EXCLUDED.authoritative_side, updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(side)
    .execute(pool)
    .await
    .expect("seed_authority");
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_authority WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn response_body(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse JSON")
}

fn get_request() -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/api/integrations/sync/authority")
        .body(Body::empty())
        .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_authority_returns_empty_array_when_no_rows() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();

    cleanup(&pool, &app_id).await;

    let app = build_app(pool.clone(), tenant_id);
    let resp = app.oneshot(get_request()).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body, Value::Array(vec![]), "empty tenant must return []");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn get_authority_returns_caller_rows_with_correct_fields() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();

    cleanup(&pool, &app_id).await;
    seed_authority(&pool, &app_id, "quickbooks", "customer", "platform").await;
    seed_authority(&pool, &app_id, "quickbooks", "invoice", "external").await;

    let app = build_app(pool.clone(), tenant_id);
    let resp = app.oneshot(get_request()).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    let items = body.as_array().expect("body is array");
    assert_eq!(items.len(), 2, "two seeded rows must be returned");

    // Ordered by provider, entity_type — customer before invoice
    assert_eq!(items[0]["entity_type"], "customer");
    assert_eq!(items[0]["authoritative_side"], "platform");
    assert!(items[0]["authority_version"].as_i64().unwrap() >= 1);

    assert_eq!(items[1]["entity_type"], "invoice");
    assert_eq!(items[1]["authoritative_side"], "external");

    // Required fields present
    for item in items {
        assert!(item.get("provider").is_some(), "provider field missing");
        assert!(item.get("entity_type").is_some(), "entity_type field missing");
        assert!(item.get("authoritative_side").is_some(), "authoritative_side field missing");
        assert!(item.get("authority_version").is_some(), "authority_version field missing");
        assert!(item.get("last_flipped_by").is_some(), "last_flipped_by field missing");
        assert!(item.get("last_flipped_at").is_some(), "last_flipped_at field missing");
    }

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn get_authority_does_not_return_other_tenant_rows() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let app_id_a = tenant_a.to_string();
    let app_id_b = tenant_b.to_string();

    cleanup(&pool, &app_id_a).await;
    cleanup(&pool, &app_id_b).await;

    // Seed rows for both tenants
    seed_authority(&pool, &app_id_a, "quickbooks", "customer", "platform").await;
    seed_authority(&pool, &app_id_b, "quickbooks", "customer", "external").await;
    seed_authority(&pool, &app_id_b, "quickbooks", "invoice", "platform").await;

    // Call with tenant A's claims
    let app = build_app(pool.clone(), tenant_a);
    let resp = app.oneshot(get_request()).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    let items = body.as_array().expect("body is array");

    assert_eq!(items.len(), 1, "tenant A must only see its own row");
    assert_eq!(items[0]["authoritative_side"], "platform",
        "must be tenant A's row, not tenant B's external row");

    cleanup(&pool, &app_id_a).await;
    cleanup(&pool, &app_id_b).await;
}

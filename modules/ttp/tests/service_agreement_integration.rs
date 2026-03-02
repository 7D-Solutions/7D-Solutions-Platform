/// Integration tests for TTP service agreements HTTP endpoint.
///
/// Requires DATABASE_URL pointing at a running TTP Postgres instance.
/// Run with: cargo test -p ttp-rs --test service_agreement_integration -- --ignored
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Extension;
use chrono::Utc;
use security::{ActorType, VerifiedClaims};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use ttp_rs::http::service_agreements::list_service_agreements;
use ttp_rs::metrics::TtpMetrics;
use ttp_rs::AppState;

/// Connect to the TTP test database.
async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_db".to_string());
    let pool = PgPool::connect(&url).await.expect("connect TTP test db");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    pool
}

/// Clean up test data for a specific tenant.
async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM ttp_service_agreements WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_customers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Build fake VerifiedClaims for a given tenant.
fn fake_claims(tenant_id: Uuid) -> VerifiedClaims {
    let now = Utc::now();
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["ttp.read".into()],
        actor_type: ActorType::User,
        issued_at: now,
        expires_at: now + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".into(),
    }
}

/// Build the HTTP app with claims injected as an Extension.
fn build_app(pool: PgPool, claims: VerifiedClaims) -> axum::Router {
    let metrics = Arc::new(TtpMetrics::new().unwrap());
    let state = Arc::new(AppState { pool, metrics });
    axum::Router::new()
        .route(
            "/api/ttp/service-agreements",
            axum::routing::get(list_service_agreements),
        )
        .layer(Extension(claims))
        .with_state(state)
}

/// Seed a customer + service agreement.
async fn seed_agreement(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
    plan_code: &str,
    amount_minor: i64,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO ttp_customers (tenant_id, party_id, status) \
         VALUES ($1, $2, 'active') ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed customer");

    sqlx::query(
        r#"INSERT INTO ttp_service_agreements
           (tenant_id, party_id, plan_code, amount_minor, currency, status, effective_from)
           VALUES ($1, $2, $3, $4, 'usd', $5, '2026-01-01')"#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .bind(plan_code)
    .bind(amount_minor)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed agreement");
}

/// Helper: send a GET to service-agreements and parse the response body.
async fn get_agreements(app: axum::Router, query: &str) -> (StatusCode, Option<serde_json::Value>) {
    let uri = if query.is_empty() {
        "/api/ttp/service-agreements".to_string()
    } else {
        format!("/api/ttp/service-agreements?{}", query)
    };

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    if status == StatusCode::OK {
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("parse response");
        (status, Some(parsed))
    } else {
        (status, None)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn list_active_agreements_default_filter() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    seed_agreement(&pool, tenant_id, party_a, "starter", 9900, "active").await;
    seed_agreement(&pool, tenant_id, party_b, "pro", 29900, "suspended").await;

    let app = build_app(pool.clone(), fake_claims(tenant_id));
    let (status, body) = get_agreements(app, "").await;

    assert_eq!(status, StatusCode::OK);
    let body = body.expect("response body");
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert_eq!(
        body["count"], 1,
        "only active agreements returned by default"
    );
    assert_eq!(body["items"][0]["plan_code"], "starter");
    assert_eq!(body["items"][0]["amount_minor"], 9900);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn list_all_agreements() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    seed_agreement(&pool, tenant_id, party_a, "starter", 9900, "active").await;
    seed_agreement(&pool, tenant_id, party_b, "pro", 29900, "cancelled").await;

    let app = build_app(pool.clone(), fake_claims(tenant_id));
    let (status, body) = get_agreements(app, "status=all").await;

    assert_eq!(status, StatusCode::OK);
    let body = body.expect("response body");
    assert_eq!(body["count"], 2, "all agreements returned with status=all");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn list_suspended_agreements() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    seed_agreement(&pool, tenant_id, party_a, "starter", 9900, "active").await;
    seed_agreement(&pool, tenant_id, party_b, "pro", 29900, "suspended").await;

    let app = build_app(pool.clone(), fake_claims(tenant_id));
    let (status, body) = get_agreements(app, "status=suspended").await;

    assert_eq!(status, StatusCode::OK);
    let body = body.expect("response body");
    assert_eq!(body["count"], 1, "only suspended agreements");
    assert_eq!(body["items"][0]["plan_code"], "pro");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn list_empty_tenant_returns_empty_array() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let app = build_app(pool.clone(), fake_claims(tenant_id));
    let (status, body) = get_agreements(app, "").await;

    assert_eq!(status, StatusCode::OK);
    let body = body.expect("response body");
    assert_eq!(body["count"], 0);
    assert!(body["items"].as_array().unwrap().is_empty());

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
async fn invalid_status_returns_400() {
    let pool = PgPool::connect_lazy(
        &std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_db".to_string()),
    )
    .expect("lazy pool");
    let tenant_id = Uuid::new_v4();

    let app = build_app(pool, fake_claims(tenant_id));
    let (status, _) = get_agreements(app, "status=invalid").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_claims_returns_401() {
    let pool = PgPool::connect_lazy(
        &std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_db".to_string()),
    )
    .expect("lazy pool");

    let metrics = Arc::new(TtpMetrics::new().unwrap());
    let state = Arc::new(AppState { pool, metrics });
    let app = axum::Router::new()
        .route(
            "/api/ttp/service-agreements",
            axum::routing::get(list_service_agreements),
        )
        .with_state(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ttp/service-agreements")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore]
async fn results_sorted_by_plan_code() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    // Seed in reverse alphabetical order
    let party_c = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    seed_agreement(&pool, tenant_id, party_c, "pro", 29900, "active").await;
    seed_agreement(&pool, tenant_id, party_a, "basic", 4900, "active").await;
    seed_agreement(&pool, tenant_id, party_b, "enterprise", 99900, "active").await;

    let app = build_app(pool.clone(), fake_claims(tenant_id));
    let (status, body) = get_agreements(app, "").await;

    assert_eq!(status, StatusCode::OK);
    let body = body.expect("response body");
    assert_eq!(body["count"], 3);

    // Sorted by plan_code: basic < enterprise < pro
    assert_eq!(body["items"][0]["plan_code"], "basic");
    assert_eq!(body["items"][1]["plan_code"], "enterprise");
    assert_eq!(body["items"][2]["plan_code"], "pro");

    cleanup(&pool, tenant_id).await;
}

//! QBO TaxRate read proxy integration tests (bd-96iab).
//!
//! Tests exercise the HTTP handler end-to-end against a real PostgreSQL DB.
//! The happy-path sandbox test also calls QBO sandbox HTTPS; it skips when
//! .qbo-tokens.json is absent or QBO_SANDBOX env var is unset.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test qbo_taxrate_test -- --nocapture
//!
//! Sandbox test additionally requires:
//!   QBO_SANDBOX=1 and .qbo-tokens.json with access_token, refresh_token, realm_id

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    routing::get,
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{
    http::qbo_taxrate::list_taxrates,
    metrics::IntegrationsMetrics,
    AppState,
};
use security::{
    claims::ActorType,
    permissions::INTEGRATIONS_READ,
    RequirePermissionsLayer, VerifiedClaims,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ── DB helpers ────────────────────────────────────────────────────────────────

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("run integrations migrations");
    pool
}

fn unique_tenant() -> String {
    format!("qbo-txr-{}", Uuid::new_v4().simple())
}

async fn cleanup_tenant(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn seed_oauth_connection(
    pool: &PgPool,
    app_id: &str,
    realm_id: &str,
    access_token: &str,
    refresh_token: &str,
) {
    std::env::set_var("OAUTH_ENCRYPTION_KEY", "test-encryption-key-for-taxrate-test");
    sqlx::query(
        r#"
        DELETE FROM integrations_oauth_connections
        WHERE app_id = $1 AND provider = 'quickbooks'
        "#,
    )
    .bind(app_id)
    .execute(pool)
    .await
    .expect("clear stale");

    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections
            (app_id, provider, realm_id,
             access_token, refresh_token,
             access_token_expires_at, refresh_token_expires_at,
             scopes_granted, connection_status)
        VALUES
            ($1, 'quickbooks', $2,
             pgp_sym_encrypt($3, 'test-encryption-key-for-taxrate-test'),
             pgp_sym_encrypt($4, 'test-encryption-key-for-taxrate-test'),
             NOW() + INTERVAL '1 hour',
             NOW() + INTERVAL '90 days',
             'com.intuit.quickbooks.accounting',
             'connected')
        ON CONFLICT (app_id, provider) DO UPDATE
            SET realm_id = EXCLUDED.realm_id,
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                access_token_expires_at = EXCLUDED.access_token_expires_at,
                connection_status = EXCLUDED.connection_status
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .bind(access_token)
    .bind(refresh_token)
    .execute(pool)
    .await
    .expect("seed oauth connection");
}

// ── Router builders ───────────────────────────────────────────────────────────

fn make_claims(app_id: &str, perms: Vec<&str>) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: Some(app_id.parse().unwrap_or_else(|_| Uuid::new_v4())),
        roles: vec![],
        perms: perms.into_iter().map(String::from).collect(),
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn build_taxrate_router_with_claims(pool: sqlx::PgPool, claims: VerifiedClaims) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    Router::new()
        .route("/api/integrations/qbo/taxrate", get(list_taxrates))
        .route_layer(RequirePermissionsLayer::new(&[INTEGRATIONS_READ]))
        .with_state(state)
        .layer(Extension(claims))
}

fn build_taxrate_router_no_permission(pool: sqlx::PgPool) -> Router {
    let claims = VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: Some(Uuid::new_v4()),
        roles: vec![],
        perms: vec![],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    };
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    Router::new()
        .route("/api/integrations/qbo/taxrate", get(list_taxrates))
        .route_layer(RequirePermissionsLayer::new(&[INTEGRATIONS_READ]))
        .with_state(state)
        .layer(Extension(claims))
}

// ── 1. Happy path — real QBO sandbox ─────────────────────────────────────────

#[tokio::test]
#[serial]
async fn qbo_taxrate_list_returns_entries_for_connected_realm() {
    dotenvy::dotenv().ok();

    if std::env::var("QBO_SANDBOX").unwrap_or_default() != "1" {
        return;
    }

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tokens_path = root.join(".qbo-tokens.json");
    if !tokens_path.exists() {
        return;
    }

    let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json");
    let tokens: serde_json::Value = serde_json::from_str(&content).expect("parse tokens");
    let access_token = tokens["access_token"].as_str().expect("access_token");
    let refresh_token = tokens["refresh_token"].as_str().expect("refresh_token");
    let realm_id = tokens["realm_id"].as_str().expect("realm_id");

    let pool = setup_db().await;
    let app_id = unique_tenant();

    seed_oauth_connection(&pool, &app_id, realm_id, access_token, refresh_token).await;

    let claims = make_claims(&app_id, vec![INTEGRATIONS_READ]);
    let router = build_taxrate_router_with_claims(pool.clone(), claims);

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/qbo/taxrate?realm_id={}",
            realm_id
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "sandbox taxrate list must return 200"
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("parse response");
    let taxrates = parsed["taxrates"].as_array().expect("taxrates array");
    assert!(
        !taxrates.is_empty(),
        "QBO sandbox must return at least one TaxRate"
    );
    assert!(
        taxrates.iter().all(|t| t["id"].is_string()),
        "every TaxRate must have an id"
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ── 2. Realm mismatch → 403 ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn qbo_taxrate_rejects_realm_mismatch() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let connected_realm = "realm-111";
    let wrong_realm = "realm-999-different";

    seed_oauth_connection(&pool, &app_id, connected_realm, "tok", "refresh").await;

    let claims = make_claims(&app_id, vec![INTEGRATIONS_READ]);
    let router = build_taxrate_router_with_claims(pool.clone(), claims);

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/qbo/taxrate?realm_id={}",
            wrong_realm
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "mismatched realm_id must return 403"
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ── 3. No QBO connection → 412 ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn qbo_taxrate_412_when_not_connected() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    // No oauth_connection seeded for this tenant

    let claims = make_claims(&app_id, vec![INTEGRATIONS_READ]);
    let router = build_taxrate_router_with_claims(pool.clone(), claims);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/qbo/taxrate?realm_id=any-realm")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::PRECONDITION_FAILED,
        "no QBO connection must return 412"
    );
}

// ── 4. Missing permission → 403 ───────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn qbo_taxrate_requires_permission() {
    let pool = setup_db().await;

    let router = build_taxrate_router_no_permission(pool);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/qbo/taxrate?realm_id=any-realm")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "caller without integrations.read must receive 403"
    );
}

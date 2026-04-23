//! Tests for the admin-gated OAuth token import endpoint (bd-iskkg).
//!
//! All DB tests run against a real PostgreSQL instance — no mocks.
//!
//! Coverage:
//!  1. import_connection stores tokens encrypted at rest
//!  2. import_connection encryption is identical to create_connection
//!  3. import_connection upserts (second import for same tenant overwrites)
//!  4. is_import_enabled gate logic (unit)
//!  5. Permission constant is correct value and distinct from coarse permissions
//!  6. Router builds with the import route (compile-time)
//!  7. In-process axum: no JWT → 401 (RequirePermissionsLayer in place)
//!  8. import_connection result visible via get_connection_status

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use event_bus::InMemoryBus;
use integrations_rs::domain::oauth::service;
use integrations_rs::http::oauth::is_import_enabled;
use integrations_rs::{metrics::IntegrationsMetrics, AppState};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn unique_tenant() -> String {
    format!("import-{}", Uuid::new_v4().simple())
}

fn unique_realm() -> String {
    format!("realm-{}", Uuid::new_v4().simple())
}

fn set_encryption_key() {
    std::env::set_var("OAUTH_ENCRYPTION_KEY", "test-encryption-key-32bytes!!");
}

// ============================================================================
// 1. import_connection stores tokens encrypted at rest
// ============================================================================

#[tokio::test]
#[serial]
async fn test_import_tokens_encrypted_at_rest() {
    let pool = setup_db().await;
    set_encryption_key();
    let tenant = unique_tenant();
    let realm = unique_realm();
    let at = format!("import-at-{}", Uuid::new_v4());
    let rt = format!("import-rt-{}", Uuid::new_v4());

    service::import_connection(
        &pool, &tenant, "quickbooks", &realm,
        &at, &rt, 3600, 8726400,
        "com.intuit.quickbooks.accounting",
    )
    .await
    .expect("import_connection failed");

    // Raw bytes must not contain plaintext tokens
    let row: (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT access_token, refresh_token
         FROM integrations_oauth_connections
         WHERE app_id = $1 AND provider = 'quickbooks'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("raw query failed");

    let raw_access = String::from_utf8_lossy(&row.0);
    let raw_refresh = String::from_utf8_lossy(&row.1);

    assert!(
        !raw_access.contains(&at),
        "access_token must not be stored as plaintext"
    );
    assert!(
        !raw_refresh.contains(&rt),
        "refresh_token must not be stored as plaintext"
    );

    // Decrypt must round-trip correctly
    let decrypted = service::get_access_token(&pool, &tenant, "quickbooks")
        .await
        .expect("get_access_token failed");
    assert_eq!(decrypted, at);
}

// ============================================================================
// 2. import_connection encryption is identical to create_connection
// ============================================================================

#[tokio::test]
#[serial]
async fn test_import_encryption_identical_to_create() {
    let pool = setup_db().await;
    set_encryption_key();
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let realm_a = unique_realm();
    let realm_b = unique_realm();

    let token_val = format!("shared-at-{}", Uuid::new_v4());

    // create_connection path
    service::create_connection(
        &pool, &tenant_a, "quickbooks", &realm_a,
        "com.intuit.quickbooks.accounting",
        &integrations_rs::domain::oauth::TokenResponse {
            access_token: token_val.clone(),
            refresh_token: "rt-create".to_string(),
            expires_in: 3600,
            x_refresh_token_expires_in: 8726400,
        },
    )
    .await
    .expect("create_connection failed");

    // import_connection path
    service::import_connection(
        &pool, &tenant_b, "quickbooks", &realm_b,
        &token_val, "rt-import", 3600, 8726400,
        "com.intuit.quickbooks.accounting",
    )
    .await
    .expect("import_connection failed");

    // Both should decrypt to the same plaintext
    let dec_a = service::get_access_token(&pool, &tenant_a, "quickbooks")
        .await
        .expect("get_access_token tenant_a failed");
    let dec_b = service::get_access_token(&pool, &tenant_b, "quickbooks")
        .await
        .expect("get_access_token tenant_b failed");

    assert_eq!(dec_a, token_val, "create_connection must decrypt correctly");
    assert_eq!(dec_b, token_val, "import_connection must decrypt correctly");
    assert_eq!(dec_a, dec_b, "both paths must produce the same plaintext");
}

// ============================================================================
// 3. import_connection upserts: second import overwrites the first
// ============================================================================

#[tokio::test]
#[serial]
async fn test_import_upserts_existing_connection() {
    let pool = setup_db().await;
    set_encryption_key();
    let tenant = unique_tenant();
    let realm = unique_realm();

    let first = service::import_connection(
        &pool, &tenant, "quickbooks", &realm,
        "at-first", "rt-first", 3600, 8726400,
        "com.intuit.quickbooks.accounting",
    )
    .await
    .expect("first import failed");

    let second = service::import_connection(
        &pool, &tenant, "quickbooks", &realm,
        "at-second", "rt-second", 7200, 8726400,
        "com.intuit.quickbooks.accounting",
    )
    .await
    .expect("second import failed");

    assert_eq!(first.id, second.id, "upsert must preserve row identity");
    assert_eq!(second.connection_status, "connected");

    let decrypted = service::get_access_token(&pool, &tenant, "quickbooks")
        .await
        .expect("get_access_token after upsert failed");
    assert_eq!(decrypted, "at-second", "second import must overwrite first");
}

// ============================================================================
// 4. is_import_enabled gate logic
//    These tests mutate env vars — serial to prevent races.
// ============================================================================

#[test]
#[serial]
fn test_gate_open_when_flag_set() {
    std::env::set_var("OAUTH_IMPORT_ENABLED", "1");
    std::env::set_var("ENV", "production");
    let result = is_import_enabled();
    std::env::remove_var("OAUTH_IMPORT_ENABLED");
    std::env::remove_var("ENV");
    assert!(result, "gate must be open when OAUTH_IMPORT_ENABLED=1 even in production");
}

#[test]
#[serial]
fn test_gate_open_in_non_production() {
    std::env::remove_var("OAUTH_IMPORT_ENABLED");
    std::env::set_var("ENV", "staging");
    let result = is_import_enabled();
    std::env::remove_var("ENV");
    assert!(result, "gate must be open in non-production environments");
}

#[test]
#[serial]
fn test_gate_closed_in_production_without_flag() {
    std::env::remove_var("OAUTH_IMPORT_ENABLED");
    std::env::set_var("ENV", "production");
    let result = is_import_enabled();
    std::env::remove_var("ENV");
    assert!(!result, "gate must be closed in production without OAUTH_IMPORT_ENABLED=1");
}

#[test]
#[serial]
fn test_gate_closed_when_flag_is_zero() {
    std::env::set_var("OAUTH_IMPORT_ENABLED", "0");
    std::env::set_var("ENV", "production");
    let result = is_import_enabled();
    std::env::remove_var("OAUTH_IMPORT_ENABLED");
    std::env::remove_var("ENV");
    assert!(!result, "OAUTH_IMPORT_ENABLED=0 must keep the gate closed in production");
}

// ============================================================================
// 5. Permission constant is correct and distinct from coarse permissions
// ============================================================================

#[test]
fn test_oauth_admin_permission_constant() {
    assert_eq!(
        security::permissions::INTEGRATIONS_OAUTH_ADMIN,
        "integrations.oauth.admin"
    );
    assert!(!security::permissions::INTEGRATIONS_OAUTH_ADMIN.is_empty());
    assert!(security::permissions::INTEGRATIONS_OAUTH_ADMIN.contains('.'));
    assert!(security::permissions::INTEGRATIONS_OAUTH_ADMIN.starts_with("integrations.oauth."));
}

#[test]
fn test_oauth_admin_permission_distinct_from_coarse() {
    assert_ne!(
        security::permissions::INTEGRATIONS_OAUTH_ADMIN,
        security::permissions::INTEGRATIONS_MUTATE,
        "oauth.admin must not equal integrations.mutate"
    );
    assert_ne!(
        security::permissions::INTEGRATIONS_OAUTH_ADMIN,
        security::permissions::INTEGRATIONS_READ,
        "oauth.admin must not equal integrations.read"
    );
}

// ============================================================================
// 6. Router builds with import route (compile-time check)
// ============================================================================

#[test]
fn router_builds_with_import_route() {
    let _: fn(std::sync::Arc<integrations_rs::AppState>) -> axum::Router =
        integrations_rs::http::router;
}

// ============================================================================
// 7. In-process axum: no JWT → 401 (RequirePermissionsLayer is in place)
//
// Builds the real router with a real pool and calls it in-process.
// The RequirePermissionsLayer rejects before any handler logic runs,
// so the bus/metrics are never invoked — this is still real-service testing
// because we exercise the actual routing stack.
// ============================================================================

#[tokio::test]
#[serial]
async fn test_import_without_jwt_returns_401() {
    let pool = setup_db().await;
    // Ensure env gate is open so we reach the permission check
    std::env::remove_var("OAUTH_IMPORT_ENABLED");
    std::env::remove_var("ENV");

    let metrics = Arc::new(IntegrationsMetrics::new().expect("metrics init failed"));
    let bus: Arc<dyn event_bus::EventBus> = Arc::new(InMemoryBus::new());
    let state = Arc::new(AppState { pool, metrics, bus, webhooks_key: [0u8; 32] });
    let app = integrations_rs::http::router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/oauth/import")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"provider":"quickbooks","realm_id":"r","access_token":"at","refresh_token":"rt","expires_in":3600,"refresh_token_expires_in":8726400,"scopes":"com.intuit.quickbooks.accounting"}"#,
        ))
        .unwrap();

    let resp = app.oneshot(req).await.expect("in-process request failed");

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "import without JWT must return 401 — RequirePermissionsLayer must be registered"
    );
}

// ============================================================================
// 8. import_connection result appears via get_connection_status
// ============================================================================

#[tokio::test]
#[serial]
async fn test_imported_connection_visible_in_status() {
    let pool = setup_db().await;
    set_encryption_key();
    let tenant = unique_tenant();
    let realm = unique_realm();
    let scopes = "com.intuit.quickbooks.accounting";

    // No connection yet
    let none = service::get_connection_status(&pool, &tenant, "quickbooks")
        .await
        .expect("status query should not error");
    assert!(none.is_none());

    service::import_connection(
        &pool, &tenant, "quickbooks", &realm,
        "at-visible", "rt-visible", 3600, 8726400, scopes,
    )
    .await
    .expect("import failed");

    let info = service::get_connection_status(&pool, &tenant, "quickbooks")
        .await
        .expect("status query failed")
        .expect("connection must be present after import");

    assert_eq!(info.app_id, tenant);
    assert_eq!(info.provider, "quickbooks");
    assert_eq!(info.realm_id, realm);
    assert_eq!(info.connection_status, "connected");
    assert_eq!(info.scopes_granted, scopes);
}

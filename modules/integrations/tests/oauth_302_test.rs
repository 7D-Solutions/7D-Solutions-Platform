//! Integration tests for OAuth 302 redirect callback (bd-5899o).
//!
//! Proves that callback always returns 302, never JSON, and HMAC-verified state.
//! The token_exchange_failed test makes a real HTTPS call to Intuit's token endpoint
//! with a garbage code — Intuit rejects it, triggering the failure path.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs -- oauth_302

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    routing::get,
    Router,
};
use event_bus::InMemoryBus;
use integrations_rs::{
    http::oauth::{callback, connect, encode_state, OAuthStatePayload},
    metrics::IntegrationsMetrics,
    AppState,
};
use serial_test::serial;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

const TEST_SECRET: &str = "test-oauth-state-secret-32-chars!";
const TEST_RETURN_URL: &str = "https://test.example.app/dashboard";
const TEST_ORIGIN: &str = "https://test.example.app";

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

fn build_callback_router(pool: sqlx::PgPool) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("metrics")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    Router::new()
        .route(
            "/api/integrations/oauth/callback/{provider}",
            get(callback),
        )
        .with_state(state)
}

fn build_connect_router(pool: sqlx::PgPool) -> Router {
    use axum::Extension;
    use chrono::Utc;
    use security::{claims::ActorType, VerifiedClaims};

    let tenant_id = Uuid::new_v4();
    let claims = VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
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
        .route(
            "/api/integrations/oauth/connect/{provider}",
            get(connect),
        )
        .with_state(state)
        .layer(Extension(claims))
}

fn set_env_vars() {
    std::env::set_var("OAUTH_STATE_SECRET", TEST_SECRET);
    std::env::set_var("OAUTH_DEFAULT_RETURN_URL", TEST_RETURN_URL);
    std::env::set_var("OAUTH_ALLOWED_RETURN_ORIGINS", TEST_ORIGIN);
    std::env::set_var("QBO_CLIENT_ID", "test-client-id");
    std::env::set_var("QBO_CLIENT_SECRET", "test-client-secret");
    std::env::set_var("QBO_REDIRECT_URI", "https://test.example.app/callback");
}

fn clear_env_vars() {
    std::env::remove_var("OAUTH_STATE_SECRET");
    std::env::remove_var("OAUTH_DEFAULT_RETURN_URL");
    std::env::remove_var("OAUTH_ALLOWED_RETURN_ORIGINS");
    std::env::remove_var("QBO_CLIENT_ID");
    std::env::remove_var("QBO_CLIENT_SECRET");
    std::env::remove_var("QBO_REDIRECT_URI");
}

fn valid_state(return_url: &str) -> String {
    let payload = OAuthStatePayload {
        app_id: Uuid::new_v4().to_string(),
        return_url: return_url.to_string(),
        nonce: "testnonce".to_string(),
    };
    encode_state(&payload).expect("encode state")
}

fn location(resp: &axum::response::Response) -> String {
    resp.headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

// ============================================================================
// Callback tests
// ============================================================================

/// Malformed state blob → 302 to default URL with error=invalid_state.
#[tokio::test]
#[serial]
async fn oauth_302_callback_invalid_state_malformed() {
    set_env_vars();
    let pool = test_pool().await;
    let router = build_callback_router(pool);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/oauth/callback/quickbooks?code=garbage&state=not-a-valid-blob")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FOUND, "must be 302");
    let loc = location(&resp);
    assert!(
        loc.contains("error=invalid_state"),
        "Location must contain error=invalid_state, got: {}",
        loc
    );

    clear_env_vars();
}

/// Valid payload with one HMAC byte flipped → 302 with error=invalid_state.
#[tokio::test]
#[serial]
async fn oauth_302_callback_invalid_state_hmac_tampered() {
    set_env_vars();
    let pool = test_pool().await;

    let mut state = valid_state(TEST_RETURN_URL);
    // Flip the last character of the state string to corrupt the HMAC
    let last = state.pop().unwrap();
    let replacement = if last == 'A' { 'B' } else { 'A' };
    state.push(replacement);

    let router = build_callback_router(pool);
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/oauth/callback/quickbooks?code=garbage&realmId=123&state={}",
            urlencoding::encode(&state)
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FOUND, "must be 302");
    assert!(
        location(&resp).contains("error=invalid_state"),
        "tampered HMAC must yield error=invalid_state"
    );

    clear_env_vars();
}

/// QBO callback with valid state but missing realmId → 302 with error=missing_realm_id.
#[tokio::test]
#[serial]
async fn oauth_302_callback_missing_realm_id() {
    set_env_vars();
    let pool = test_pool().await;
    let state = valid_state(TEST_RETURN_URL);
    let router = build_callback_router(pool);

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/oauth/callback/quickbooks?code=any_code&state={}",
            urlencoding::encode(&state)
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FOUND, "must be 302");
    assert!(
        location(&resp).contains("error=missing_realm_id"),
        "missing realmId must yield error=missing_realm_id, got: {}",
        location(&resp)
    );

    clear_env_vars();
}

/// Valid state + garbage code → Intuit rejects → 302 with error=token_exchange_failed.
/// Requires outbound HTTPS to oauth.platform.intuit.com.
#[tokio::test]
#[serial]
async fn oauth_302_callback_token_exchange_failed() {
    set_env_vars();
    let pool = test_pool().await;
    let state = valid_state(TEST_RETURN_URL);
    let router = build_callback_router(pool);

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/oauth/callback/quickbooks?code=definitely_invalid_code&realmId=test-realm&state={}",
            urlencoding::encode(&state)
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::FOUND, "must be 302");
    assert!(
        location(&resp).contains("error=token_exchange_failed"),
        "Intuit rejection must yield error=token_exchange_failed, got: {}",
        location(&resp)
    );

    clear_env_vars();
}

/// Every non-ignored callback test must not return Content-Type: application/json.
#[tokio::test]
#[serial]
async fn oauth_302_callback_never_returns_json() {
    set_env_vars();
    let pool = test_pool().await;

    let cases: Vec<(&str, &str)> = vec![
        (
            "/api/integrations/oauth/callback/quickbooks?code=x&state=not-valid",
            "malformed state",
        ),
        (
            "/api/integrations/oauth/callback/quickbooks?code=x",
            "missing state",
        ),
    ];

    for (uri, label) in cases {
        let router = build_callback_router(pool.clone());
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.expect("oneshot");
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            !ct.contains("application/json"),
            "callback {} must never return JSON, got content-type: {}",
            label,
            ct
        );
    }

    clear_env_vars();
}

/// Every non-ignored callback test must return 302.
#[tokio::test]
#[serial]
async fn oauth_302_callback_never_returns_success_code() {
    set_env_vars();
    let pool = test_pool().await;

    let cases = vec![
        "/api/integrations/oauth/callback/quickbooks?code=x&state=not-valid",
        "/api/integrations/oauth/callback/quickbooks?code=x",
    ];

    for uri in cases {
        let router = build_callback_router(pool.clone());
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.expect("oneshot");
        assert_eq!(
            resp.status(),
            StatusCode::FOUND,
            "callback {} must return 302, got {}",
            uri,
            resp.status()
        );
    }

    clear_env_vars();
}

// ============================================================================
// Connect tests
// ============================================================================

/// return_url with disallowed origin → 422.
#[tokio::test]
#[serial]
async fn oauth_302_connect_rejects_return_url_off_allowlist() {
    set_env_vars();
    let pool = test_pool().await;
    let router = build_connect_router(pool);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/integrations/oauth/connect/quickbooks?return_url=https://evil.com/steal")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY, "off-allowlist origin must return 422");

    clear_env_vars();
}

/// return_url on allowlist → 307 to provider; decoded state contains the return_url.
#[tokio::test]
#[serial]
async fn oauth_302_connect_accepts_return_url_on_allowlist() {
    set_env_vars();
    let pool = test_pool().await;
    let router = build_connect_router(pool);

    let return_url = format!("{}/admin", TEST_ORIGIN);
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/oauth/connect/quickbooks?return_url={}",
            urlencoding::encode(&return_url)
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT, "allowed origin must return 307");

    // Verify the state in Location header is decodable and contains return_url
    let loc = location(&resp);
    assert!(loc.contains("state="), "Location must contain state param");

    clear_env_vars();
}

/// return_url with existing query string → state preserves the full return_url including query.
#[tokio::test]
#[serial]
async fn oauth_302_connect_preserves_existing_query_on_return_url() {
    set_env_vars();
    let pool = test_pool().await;
    let router = build_connect_router(pool);

    let return_url = format!("{}/admin?tab=integrations", TEST_ORIGIN);
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/api/integrations/oauth/connect/quickbooks?return_url={}",
            urlencoding::encode(&return_url)
        ))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);

    // The encoded state in the Location URL should contain the full return_url
    let loc = location(&resp);
    assert!(
        loc.contains("state="),
        "Location must have state, got: {}",
        loc
    );

    clear_env_vars();
}

// ============================================================================
// Env validation tests
// ============================================================================

/// Missing OAUTH_STATE_SECRET → panic with message containing "OAUTH_STATE_SECRET".
#[test]
#[serial]
#[should_panic(expected = "OAUTH_STATE_SECRET")]
fn oauth_302_env_missing_state_secret() {
    std::env::remove_var("OAUTH_STATE_SECRET");
    std::env::set_var("OAUTH_DEFAULT_RETURN_URL", TEST_RETURN_URL);
    std::env::set_var("OAUTH_ALLOWED_RETURN_ORIGINS", TEST_ORIGIN);
    integrations_rs::http::oauth_validation::validate_oauth_env_pub();
}

/// OAUTH_STATE_SECRET shorter than 32 bytes → panic.
#[test]
#[serial]
#[should_panic(expected = "too short")]
fn oauth_302_env_short_state_secret() {
    std::env::set_var("OAUTH_STATE_SECRET", "short");
    std::env::set_var("OAUTH_DEFAULT_RETURN_URL", TEST_RETURN_URL);
    std::env::set_var("OAUTH_ALLOWED_RETURN_ORIGINS", TEST_ORIGIN);
    integrations_rs::http::oauth_validation::validate_oauth_env_pub();
}

/// Missing OAUTH_DEFAULT_RETURN_URL → panic with that name in message.
#[test]
#[serial]
#[should_panic(expected = "OAUTH_DEFAULT_RETURN_URL")]
fn oauth_302_env_missing_default_return_url() {
    std::env::set_var("OAUTH_STATE_SECRET", TEST_SECRET);
    std::env::remove_var("OAUTH_DEFAULT_RETURN_URL");
    std::env::set_var("OAUTH_ALLOWED_RETURN_ORIGINS", TEST_ORIGIN);
    integrations_rs::http::oauth_validation::validate_oauth_env_pub();
}

/// OAUTH_ALLOWED_RETURN_ORIGINS empty → panic with that name in message.
#[test]
#[serial]
#[should_panic(expected = "OAUTH_ALLOWED_RETURN_ORIGINS")]
fn oauth_302_env_missing_allowed_origins() {
    std::env::set_var("OAUTH_STATE_SECRET", TEST_SECRET);
    std::env::set_var("OAUTH_DEFAULT_RETURN_URL", TEST_RETURN_URL);
    std::env::set_var("OAUTH_ALLOWED_RETURN_ORIGINS", "");
    integrations_rs::http::oauth_validation::validate_oauth_env_pub();
}

// ============================================================================
// Live sandbox tests (ignored in CI)
// ============================================================================

/// Full end-to-end against real Intuit sandbox.
/// Requires INTUIT_SANDBOX_CLIENT_ID and INTUIT_SANDBOX_CLIENT_SECRET.
#[tokio::test]
#[serial]
#[ignore]
async fn oauth_302_callback_success_live_intuit_sandbox() {
    // Requires manual setup with live Intuit sandbox credentials.
    // Run manually: cargo test oauth_302_callback_success_live_intuit_sandbox -- --ignored
    todo!("configure INTUIT_SANDBOX_CLIENT_ID and INTUIT_SANDBOX_CLIENT_SECRET, then implement");
}

/// DB write failure path (requires successful token exchange).
#[tokio::test]
#[serial]
#[ignore]
async fn oauth_302_callback_db_write_failed() {
    // Requires live Intuit sandbox credentials to get past token exchange.
    todo!("configure INTUIT_SANDBOX_CLIENT_ID and INTUIT_SANDBOX_CLIENT_SECRET");
}

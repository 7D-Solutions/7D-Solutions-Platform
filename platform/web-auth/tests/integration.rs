//! Integration tests for platform-web-auth against a real identity-auth service.
//!
//! Requires 7d-auth-lb running at IDENTITY_AUTH_URL (default: http://localhost:8080)
//! and a provisioned tenant at TEST_TENANT_ID (default: Huber dev tenant).
//!
//! Tests are skipped when the service is unreachable.
//!
//! Run: ./scripts/cargo-slot.sh test -p platform-web-auth --test integration -- --nocapture

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::Extension;
use axum::routing::get;
use axum::{Json, Router};
use reqwest::cookie::Jar;
use security::claims::{JwtVerifier, VerifiedClaims};
use uuid::Uuid;

// ── Env ───────────────────────────────────────────────────────────────────────

fn auth_url() -> String {
    std::env::var("IDENTITY_AUTH_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

/// Provisioned active tenant for all integration tests.
/// Defaults to the Huber dev tenant which is always active in the dev stack.
fn test_tenant_id() -> Uuid {
    std::env::var("TEST_TENANT_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .unwrap_or_else(|| {
            Uuid::parse_str("067abf94-5c9d-5a82-b32c-78e5adb65ddd").expect("Huber tenant UUID")
        })
}

// ── Service reachability + JWT key ───────────────────────────────────────────

/// Fetch and cache the JWT public key PEM from the running auth service JWKS endpoint.
static JWT_VERIFIER: OnceLock<Option<Arc<JwtVerifier>>> = OnceLock::new();

async fn get_jwt_verifier() -> Option<Arc<JwtVerifier>> {
    JWT_VERIFIER
        .get_or_init(|| {
            // Blocking init runs once; tests are async but OnceLock init is sync.
            // Use std::thread to run a tokio runtime for the async fetch.
            let auth = auth_url();
            let result: Option<Arc<JwtVerifier>> = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                rt.block_on(async {
                    let jwks_url = format!("{}/api/auth/jwks", auth);
                    JwtVerifier::from_jwks_url(&jwks_url, Duration::from_secs(300), false)
                        .await
                        .ok()
                        .map(Arc::new)
                })
            })
            .join()
            .ok()
            .flatten();
            result
        })
        .clone()
}

/// Returns None when identity-auth is unreachable (caller should skip the test).
async fn check_service_reachable() -> Option<Arc<JwtVerifier>> {
    let verifier = get_jwt_verifier().await?;
    Some(verifier)
}

// ── Test user factory ─────────────────────────────────────────────────────────

/// Register a unique test user in the provisioned active tenant.
async fn register_test_user() -> (String, String, Uuid) {
    let tenant_id = test_tenant_id();
    let user_id = Uuid::new_v4();
    let email = format!("web-auth-{}@test.local", Uuid::new_v4());
    let password = format!("TestWebAuth1234-{}", &Uuid::new_v4().to_string()[..8]);

    let resp = reqwest::Client::new()
        .post(format!("{}/api/auth/register", auth_url()))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "tenant_id": tenant_id,
            "user_id": user_id,
        }))
        .send()
        .await
        .expect("register request failed");

    assert!(
        resp.status().is_success(),
        "register failed ({}): {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    (email, password, tenant_id)
}

// ── Test app builder ──────────────────────────────────────────────────────────

/// Build an in-process test app with WebAuthProxy nested under /api/auth,
/// plus a protected /api/things route that requires Extension<VerifiedClaims>.
async fn build_test_app(prefix: &str, verifier: Arc<JwtVerifier>) -> (SocketAddr, reqwest::Client) {
    use platform_web_auth::WebAuthProxy;

    let (auth_router, cookie_mw) = WebAuthProxy::builder()
        .cookie_prefix(prefix)
        .refresh_cookie_path("/api/auth")
        .identity_auth_url(auth_url())
        .access_cookie_max_age(Duration::from_secs(60 * 30))
        .refresh_cookie_max_age(Duration::from_secs(60 * 60 * 24 * 30))
        .with_verifier(verifier)
        .build()
        .expect("WebAuthProxy::build");

    let app = Router::new()
        .route(
            "/api/things",
            get(
                |Extension(claims): Extension<VerifiedClaims>| async move {
                    Json(serde_json::json!({"user_id": claims.user_id, "ok": true}))
                },
            ),
        )
        .nest("/api/auth", auth_router)
        .layer(cookie_mw);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let cookie_jar = Arc::new(Jar::default());
    let client = reqwest::Client::builder()
        .cookie_provider(cookie_jar)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client");

    (addr, client)
}

fn base(addr: SocketAddr) -> String {
    format!("http://{}", addr)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn login_sets_httponly_cookies_with_correct_attributes() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (email, password, tenant_id) = register_test_user().await;
    let (addr, _) = build_test_app("tst", verifier).await;

    // Use a raw client (no jar) so we can inspect response headers directly
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/auth/login", base(addr)))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "tenant_id": tenant_id,
        }))
        .send()
        .await
        .expect("login request");

    assert_eq!(
        resp.status(),
        200,
        "login should succeed: {}",
        resp.text().await.unwrap_or_default()
    );

    let cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();

    assert_eq!(cookies.len(), 2, "expected 2 Set-Cookie headers, got: {cookies:?}");

    let has_session = cookies.iter().any(|c| c.starts_with("tst_session="));
    let has_refresh = cookies.iter().any(|c| c.starts_with("tst_refresh="));
    assert!(has_session, "missing tst_session cookie: {cookies:?}");
    assert!(has_refresh, "missing tst_refresh cookie: {cookies:?}");

    for cookie in &cookies {
        let lc = cookie.to_lowercase();
        assert!(lc.contains("httponly"), "HttpOnly missing: {cookie}");
        assert!(lc.contains("samesite=lax"), "SameSite=Lax missing: {cookie}");
        assert!(lc.contains("max-age="), "Max-Age missing: {cookie}");
        assert!(!lc.contains("; secure"), "Secure must be absent in dev: {cookie}");
    }

    let session = cookies.iter().find(|c| c.starts_with("tst_session=")).unwrap();
    assert!(
        session.to_lowercase().contains("path=/;") || session.to_lowercase().contains("path=/,"),
        "access cookie must have Path=/: {session}"
    );

    let refresh = cookies.iter().find(|c| c.starts_with("tst_refresh=")).unwrap();
    assert!(
        refresh.to_lowercase().contains("path=/api/auth"),
        "refresh cookie must have Path=/api/auth: {refresh}"
    );
}

#[tokio::test]
async fn secure_flag_absent_when_app_env_unset() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let prev = std::env::var("APP_ENV").ok();
    std::env::remove_var("APP_ENV");

    let (email, password, tenant_id) = register_test_user().await;
    let (addr, _) = build_test_app("nosec", verifier).await;

    let resp = reqwest::Client::new()
        .post(format!("{}/api/auth/login", base(addr)))
        .json(&serde_json::json!({"email": email, "password": password, "tenant_id": tenant_id}))
        .send()
        .await
        .expect("login");

    assert_eq!(resp.status(), 200, "login failed: {}", resp.text().await.unwrap_or_default());

    let cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();

    for cookie in &cookies {
        assert!(
            !cookie.to_lowercase().contains("; secure"),
            "Secure attribute must be absent when APP_ENV is unset: {cookie}"
        );
    }

    if let Some(v) = prev {
        std::env::set_var("APP_ENV", v);
    }
}

#[tokio::test]
async fn force_secure_overrides_app_env() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    std::env::remove_var("APP_ENV");
    let (email, password, tenant_id) = register_test_user().await;

    let (auth_router, cookie_mw) = platform_web_auth::WebAuthProxy::builder()
        .cookie_prefix("fsec")
        .refresh_cookie_path("/api/auth")
        .identity_auth_url(auth_url())
        .force_secure(true)
        .with_verifier(verifier)
        .build()
        .expect("build");

    let app = Router::new()
        .nest("/api/auth", auth_router)
        .layer(cookie_mw);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let resp = reqwest::Client::new()
        .post(format!("http://{}/api/auth/login", addr))
        .json(&serde_json::json!({"email": email, "password": password, "tenant_id": tenant_id}))
        .send()
        .await
        .expect("login");

    assert_eq!(resp.status(), 200, "login failed: {}", resp.text().await.unwrap_or_default());

    let cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();

    assert_eq!(cookies.len(), 2, "expected 2 cookies: {cookies:?}");
    for cookie in &cookies {
        assert!(
            cookie.to_lowercase().contains("; secure"),
            "Secure attribute must be present with force_secure(true): {cookie}"
        );
    }
}

#[tokio::test]
async fn me_returns_claims_with_valid_access_cookie() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (email, password, tenant_id) = register_test_user().await;
    let (addr, client) = build_test_app("me", verifier).await;

    // Login to plant cookies in the jar
    let login_resp = client
        .post(format!("{}/api/auth/login", base(addr)))
        .json(&serde_json::json!({"email": email, "password": password, "tenant_id": tenant_id}))
        .send()
        .await
        .expect("login");
    assert_eq!(login_resp.status(), 200, "login failed: {}", login_resp.text().await.unwrap_or_default());

    let me_resp = client
        .get(format!("{}/api/auth/me", base(addr)))
        .send()
        .await
        .expect("me request");

    assert_eq!(me_resp.status(), 200, "me should return 200 with valid cookie");

    let body: serde_json::Value = me_resp.json().await.expect("me body");
    assert!(!body["user_id"].is_null(), "user_id missing from /me response");
    assert_eq!(
        body["tenant_id"].as_str().unwrap_or(""),
        tenant_id.to_string(),
        "tenant_id mismatch"
    );
}

#[tokio::test]
async fn me_returns_401_without_cookie() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (addr, _) = build_test_app("me401", verifier).await;

    let resp = reqwest::Client::new()
        .get(format!("{}/api/auth/me", base(addr)))
        .send()
        .await
        .expect("me request");

    assert_eq!(resp.status(), 401, "me without cookie must return 401");
    let body: serde_json::Value = resp.json().await.expect("body");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn cookie_middleware_attaches_claims_to_downstream_route() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (email, password, tenant_id) = register_test_user().await;
    let (addr, client) = build_test_app("mw", verifier).await;

    let login_resp = client
        .post(format!("{}/api/auth/login", base(addr)))
        .json(&serde_json::json!({"email": email, "password": password, "tenant_id": tenant_id}))
        .send()
        .await
        .expect("login");
    assert_eq!(login_resp.status(), 200, "login failed: {}", login_resp.text().await.unwrap_or_default());

    // /api/things extracts Extension<VerifiedClaims>; returns 500 if middleware didn't attach claims
    let resp = client
        .get(format!("{}/api/things", base(addr)))
        .send()
        .await
        .expect("things request");

    assert_eq!(
        resp.status(),
        200,
        "downstream route should see VerifiedClaims from cookie middleware"
    );
    let body: serde_json::Value = resp.json().await.expect("things body");
    assert_eq!(body["ok"], true);
    assert!(!body["user_id"].is_null());
}

#[tokio::test]
async fn cookie_middleware_passes_through_silently_without_cookie() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (addr, _) = build_test_app("pt", verifier).await;

    // No cookie — /api/things will 500 (missing Extension) but middleware must NOT 401 itself
    let resp = reqwest::Client::new()
        .get(format!("{}/api/things", base(addr)))
        .send()
        .await
        .expect("things request");

    assert_ne!(
        resp.status(),
        401,
        "middleware must pass through silently — not return 401 itself"
    );
    assert_ne!(
        resp.status(),
        403,
        "middleware must not return 403 — that's the downstream layer's job"
    );
}

#[tokio::test]
async fn refresh_rotates_both_cookies() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (email, password, tenant_id) = register_test_user().await;
    let (addr, client) = build_test_app("ref", verifier).await;

    let login_resp = client
        .post(format!("{}/api/auth/login", base(addr)))
        .json(&serde_json::json!({"email": email, "password": password, "tenant_id": tenant_id}))
        .send()
        .await
        .expect("login");
    assert_eq!(login_resp.status(), 200, "login failed: {}", login_resp.text().await.unwrap_or_default());

    // Capture /me tenant before refresh to confirm stability
    let me_before: serde_json::Value = client
        .get(format!("{}/api/auth/me", base(addr)))
        .send()
        .await
        .expect("me before")
        .json()
        .await
        .expect("me before json");

    let refresh_resp = client
        .post(format!("{}/api/auth/refresh", base(addr)))
        .send()
        .await
        .expect("refresh request");

    assert_eq!(
        refresh_resp.status(),
        200,
        "refresh should succeed: {}",
        refresh_resp.text().await.unwrap_or_default()
    );

    let refresh_body: serde_json::Value = refresh_resp.json().await.expect("refresh body");
    assert!(
        refresh_body["access_token"].is_string(),
        "refresh must return access_token in body"
    );

    // /me should still work with the rotated access cookie
    let me_after_resp = client
        .get(format!("{}/api/auth/me", base(addr)))
        .send()
        .await
        .expect("me after refresh");

    assert_eq!(me_after_resp.status(), 200, "me must work after refresh");

    let me_after: serde_json::Value = me_after_resp.json().await.expect("me after json");
    assert_eq!(
        me_after["tenant_id"], me_before["tenant_id"],
        "tenant_id must be stable across refresh"
    );
}

#[tokio::test]
async fn refresh_without_refresh_cookie_returns_401() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (addr, _) = build_test_app("noref", verifier).await;

    let resp = reqwest::Client::new()
        .post(format!("{}/api/auth/refresh", base(addr)))
        .send()
        .await
        .expect("refresh without cookie");

    assert_eq!(resp.status(), 401, "refresh without cookie must return 401");
    let body: serde_json::Value = resp.json().await.expect("body");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn logout_clears_cookies_and_session_is_gone() {
    let Some(verifier) = check_service_reachable().await else {
        eprintln!("SKIP: identity-auth unreachable at {}", auth_url());
        return;
    };

    let (email, password, tenant_id) = register_test_user().await;
    let (addr, client) = build_test_app("lo", verifier).await;

    // Login
    let login_resp = client
        .post(format!("{}/api/auth/login", base(addr)))
        .json(&serde_json::json!({"email": email, "password": password, "tenant_id": tenant_id}))
        .send()
        .await
        .expect("login");
    assert_eq!(login_resp.status(), 200, "login failed: {}", login_resp.text().await.unwrap_or_default());

    // Verify /me works pre-logout
    let me_before = client
        .get(format!("{}/api/auth/me", base(addr)))
        .send()
        .await
        .expect("me before logout");
    assert_eq!(me_before.status(), 200, "me must work before logout");

    // Logout
    let logout_resp = client
        .post(format!("{}/api/auth/logout", base(addr)))
        .send()
        .await
        .expect("logout");
    assert_eq!(logout_resp.status(), 200, "logout should succeed");

    let cookies: Vec<String> = logout_resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();

    // Both cookies must be cleared (Max-Age=0)
    assert_eq!(cookies.len(), 2, "logout must set 2 Set-Cookie headers: {cookies:?}");
    for cookie in &cookies {
        assert!(
            cookie.to_lowercase().contains("max-age=0"),
            "cookie must have Max-Age=0 on logout: {cookie}"
        );
    }

    // After clearing the jar, /me must return 401
    let fresh_jar = Arc::new(Jar::default());
    let fresh_client = reqwest::Client::builder()
        .cookie_provider(fresh_jar)
        .build()
        .expect("fresh client");

    let me_after = fresh_client
        .get(format!("{}/api/auth/me", base(addr)))
        .send()
        .await
        .expect("me after logout");
    assert_eq!(me_after.status(), 401, "me must return 401 after logout");
}

//! Optional CSRF middleware using the double-submit cookie pattern.
//!
//! When enabled via [`ModuleBuilder::csrf_protection`]:
//! - **GET / HEAD / OPTIONS**: sets a `__csrf` cookie containing a 32-byte
//!   random token encoded as base64url. Existing cookies are replaced on
//!   each safe request so tokens stay fresh.
//! - **POST / PUT / PATCH / DELETE**: requires an `X-CSRF-Token` header whose
//!   value matches the `__csrf` cookie exactly. Mismatch → `403 Forbidden`.
//!
//! ## Why double-submit (stateless)?
//! The cookie value is never stored server-side. The same-origin policy means
//! only JavaScript served from the same origin can read the cookie and copy it
//! into the header — cross-origin attackers cannot replicate the header even if
//! they can trigger a cookie-bearing request. `SameSite=Strict` is the primary
//! CSRF defence; the double-submit token adds defence-in-depth.
//!
//! ## Cookie attributes
//! - `HttpOnly=false` — **required** so JavaScript can read the token.
//! - `SameSite=Strict` — primary CSRF protection.
//! - `Secure` — added when `CSRF_SECURE=true` or `APP_ENV=production`.
//! - `Path=/` — cookie is sent on all paths.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;

/// Configuration for the CSRF middleware.
#[derive(Debug, Clone)]
pub struct CsrfConfig {
    /// When `true`, the `__csrf` cookie is flagged `Secure`.
    /// Enable this in production (HTTPS-only) deployments.
    pub secure: bool,
}

impl CsrfConfig {
    /// Derive config from the environment.
    ///
    /// Sets `secure = true` when either:
    /// - `CSRF_SECURE=true` or `CSRF_SECURE=1`
    /// - `APP_ENV=production` (case-insensitive)
    pub fn from_env() -> Self {
        let from_csrf_secure = std::env::var("CSRF_SECURE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let from_app_env = std::env::var("APP_ENV")
            .map(|v| v.eq_ignore_ascii_case("production"))
            .unwrap_or(false);
        Self {
            secure: from_csrf_secure || from_app_env,
        }
    }
}

/// Generate a cryptographically random CSRF token.
///
/// Returns 32 random bytes encoded as a URL-safe base64 string (no padding),
/// yielding a 43-character token.
pub fn generate_csrf_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Extract the `__csrf` cookie value from request headers.
fn get_csrf_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    for value in headers.get_all(axum::http::header::COOKIE) {
        if let Ok(s) = value.to_str() {
            for part in s.split(';') {
                let part = part.trim();
                if let Some(token) = part.strip_prefix("__csrf=") {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

/// Build the `Set-Cookie` header value for the CSRF cookie.
fn build_set_cookie(token: &str, secure: bool) -> String {
    // HttpOnly is intentionally absent — JS must read the token.
    let mut parts = vec![
        format!("__csrf={token}"),
        "Path=/".to_string(),
        "SameSite=Strict".to_string(),
    ];
    if secure {
        parts.push("Secure".to_string());
    }
    parts.join("; ")
}

/// Axum middleware function implementing the double-submit CSRF check.
///
/// Register this via [`ModuleBuilder::csrf_protection`] rather than directly.
pub async fn csrf_middleware(
    State(config): State<Arc<CsrfConfig>>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if is_safe_method(req.method()) {
        // Set a fresh token on every safe request.
        let token = generate_csrf_token();
        let cookie_value = build_set_cookie(&token, config.secure);
        let mut response = next.run(req).await;
        if let Ok(header_val) = cookie_value.parse() {
            response
                .headers_mut()
                .insert(axum::http::header::SET_COOKIE, header_val);
        }
        response
    } else {
        // Unsafe method: require header == cookie.
        let cookie_token = get_csrf_cookie(req.headers());
        let header_token = req
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        match (cookie_token, header_token) {
            (Some(cookie), Some(header)) if cookie == header => next.run(req).await,
            _ => (StatusCode::FORBIDDEN, "CSRF validation failed\n").into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use axum::routing::{get, post, put};
    use axum::Router;
    use tower::ServiceExt;

    fn make_app() -> Router {
        let config = Arc::new(CsrfConfig { secure: false });
        Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/submit", post(|| async { "ok" }))
            .route("/update", put(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                config,
                csrf_middleware,
            ))
    }

    fn extract_csrf_set_cookie(response: &axum::response::Response) -> Option<String> {
        for value in response.headers().get_all(axum::http::header::SET_COOKIE) {
            if let Ok(s) = value.to_str() {
                if s.contains("__csrf=") {
                    return Some(s.to_string());
                }
            }
        }
        None
    }

    fn extract_csrf_token(response: &axum::response::Response) -> Option<String> {
        for value in response.headers().get_all(axum::http::header::SET_COOKIE) {
            if let Ok(s) = value.to_str() {
                for part in s.split(';') {
                    let part = part.trim();
                    if let Some(token) = part.strip_prefix("__csrf=") {
                        return Some(token.to_string());
                    }
                }
            }
        }
        None
    }

    #[tokio::test]
    async fn get_sets_csrf_cookie() {
        let app = make_app();
        let req = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        assert_eq!(response.status(), StatusCode::OK);
        let token = extract_csrf_token(&response);
        assert!(token.is_some(), "GET must set the __csrf cookie");
        assert!(!token.expect("test assertion").is_empty(), "token must not be empty");
    }

    #[tokio::test]
    async fn post_without_token_returns_403() {
        let app = make_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/submit")
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn post_with_valid_token_passes() {
        let token = generate_csrf_token();
        let app = make_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/submit")
            .header("Cookie", format!("__csrf={token}"))
            .header("X-CSRF-Token", &token)
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn post_with_wrong_token_returns_403() {
        let cookie_token = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let wrong_token = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let app = make_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/submit")
            .header("Cookie", format!("__csrf={cookie_token}"))
            .header("X-CSRF-Token", wrong_token)
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn post_with_header_only_no_cookie_returns_403() {
        let token = generate_csrf_token();
        let app = make_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/submit")
            .header("X-CSRF-Token", &token)
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn put_with_valid_token_passes() {
        let token = generate_csrf_token();
        let app = make_app();
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/update")
            .header("Cookie", format!("__csrf={token}"))
            .header("X-CSRF-Token", &token)
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cookie_is_not_httponly() {
        let app = make_app();
        let req = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        let set_cookie = extract_csrf_set_cookie(&response)
            .expect("Set-Cookie must be present on GET");
        assert!(
            !set_cookie.to_lowercase().contains("httponly"),
            "cookie must NOT be HttpOnly — JS needs to read the token"
        );
    }

    #[tokio::test]
    async fn cookie_has_samesite_strict() {
        let app = make_app();
        let req = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        let set_cookie = extract_csrf_set_cookie(&response)
            .expect("Set-Cookie must be present on GET");
        assert!(
            set_cookie.contains("SameSite=Strict"),
            "cookie must have SameSite=Strict"
        );
    }

    #[tokio::test]
    async fn secure_flag_present_when_config_is_secure() {
        let config = Arc::new(CsrfConfig { secure: true });
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(config, csrf_middleware));

        let req = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        let set_cookie = extract_csrf_set_cookie(&response)
            .expect("Set-Cookie must be present on GET");
        assert!(
            set_cookie.contains("Secure"),
            "Secure flag must appear when config.secure = true"
        );
    }

    #[tokio::test]
    async fn secure_flag_absent_when_config_not_secure() {
        let app = make_app(); // secure: false
        let req = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
            .expect("test assertion");
        let response = app.oneshot(req).await.expect("test assertion");
        let set_cookie = extract_csrf_set_cookie(&response)
            .expect("Set-Cookie must be present on GET");
        // The cookie value itself could contain "Secure" by chance, but not as a
        // distinct attribute — check for the "; Secure" attribute form.
        let parts: Vec<&str> = set_cookie.split(';').collect();
        let has_secure_attr = parts
            .iter()
            .skip(1)
            .any(|p| p.trim().eq_ignore_ascii_case("secure"));
        assert!(
            !has_secure_attr,
            "Secure attribute must NOT appear when config.secure = false"
        );
    }

    #[tokio::test]
    async fn generate_csrf_token_produces_unique_values() {
        let t1 = generate_csrf_token();
        let t2 = generate_csrf_token();
        assert_ne!(t1, t2, "tokens must be unique");
        assert_eq!(t1.len(), 43, "32-byte base64url token should be 43 chars");
    }
}

//! Centralized security middleware for Axum routers.
//!
//! Provides request body size limits, request timeouts, and IP-based rate limiting.
//! All modules should apply these layers for consistent security posture.

use crate::ratelimit::{RateLimitConfig, RateLimiter};
use axum::{extract::ConnectInfo, http::StatusCode, response::IntoResponse};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Default maximum request body size: 2 MiB.
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Default request timeout: 30 seconds.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Default rate limit: 200 requests per 60 seconds per IP.
pub const DEFAULT_RATE_LIMIT: u32 = 200;

/// Default rate limit window.
pub const DEFAULT_RATE_WINDOW: Duration = Duration::from_secs(60);

/// Create a RateLimiter pre-configured with platform defaults.
pub fn default_rate_limiter() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::with_configs(
        RateLimitConfig::new(DEFAULT_RATE_LIMIT, DEFAULT_RATE_WINDOW),
        RateLimitConfig::new(DEFAULT_RATE_LIMIT / 10, DEFAULT_RATE_WINDOW),
    ))
}

/// Rate-limiting middleware for Axum routers.
///
/// Reads `Extension<Arc<RateLimiter>>` and `ConnectInfo<SocketAddr>` from the
/// request. If the rate limiter rejects the request, returns 429.
///
/// Usage in main.rs:
/// ```ignore
/// use security::middleware::{rate_limit_middleware, default_rate_limiter};
/// let limiter = default_rate_limiter();
/// let app = Router::new()
///     .route(...)
///     .layer(axum::middleware::from_fn(rate_limit_middleware))
///     .layer(Extension(limiter));
/// ```
pub async fn rate_limit_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Borrow limiter from extensions (avoids Arc clone per request).
    // All rate-limit work completes before request is moved into next.run().
    let rejected = if let Some(limiter) = request.extensions().get::<Arc<RateLimiter>>() {
        let ip = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().to_string())
            .unwrap_or_else(|| "unknown".into());
        limiter.check_limit(&ip, request.uri().path()).is_err()
    } else {
        false
    };

    if rejected {
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded\n").into_response();
    }

    next.run(request).await
}

/// Timeout middleware for Axum routers.
///
/// Wraps the inner handler in a tokio timeout. Returns 408 on expiry.
pub async fn timeout_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => (StatusCode::REQUEST_TIMEOUT, "Request timeout\n").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ratelimit::RateLimitConfig;

    #[test]
    fn test_default_rate_limiter_creates_valid_instance() {
        let limiter = default_rate_limiter();
        // Should allow requests up to the limit
        for _ in 0..DEFAULT_RATE_LIMIT {
            assert!(limiter.check_limit("127.0.0.1", "/api/test").is_ok());
        }
        // Next request should be rejected
        assert!(limiter.check_limit("127.0.0.1", "/api/test").is_err());
    }

    #[test]
    fn test_security_constants_are_sane() {
        assert_eq!(DEFAULT_BODY_LIMIT, 2 * 1024 * 1024);
        assert_eq!(DEFAULT_REQUEST_TIMEOUT, Duration::from_secs(30));
        assert_eq!(DEFAULT_RATE_LIMIT, 200);
        assert_eq!(DEFAULT_RATE_WINDOW, Duration::from_secs(60));
    }

    #[test]
    fn test_default_rate_limiter_ip_isolation() {
        let limiter = default_rate_limiter();
        // Exhaust quota for IP 1
        for _ in 0..DEFAULT_RATE_LIMIT {
            limiter
                .check_limit("10.0.0.1", "/api/test")
                .expect("within quota");
        }
        assert!(limiter.check_limit("10.0.0.1", "/api/test").is_err());

        // IP 2 should still have full quota
        assert!(limiter.check_limit("10.0.0.2", "/api/test").is_ok());
    }

    #[tokio::test]
    async fn test_timeout_middleware_passes_fast_requests() {
        use axum::{body::Body, routing::get, Router};
        use http::Request;
        use tower::ServiceExt;

        let app = Router::new()
            .route("/fast", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(timeout_middleware));

        let req = Request::builder()
            .uri("/fast")
            .body(Body::empty())
            .expect("build request");

        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_rejects_over_quota() {
        use axum::{body::Body, routing::get, Extension, Router};
        use http::Request;
        use tower::ServiceExt;

        let limiter = Arc::new(RateLimiter::with_configs(
            RateLimitConfig::new(2, Duration::from_secs(60)),
            RateLimitConfig::new(1, Duration::from_secs(60)),
        ));

        let app = Router::new()
            .route("/api/test", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(rate_limit_middleware))
            .layer(Extension(limiter));

        // First 2 requests should succeed
        for _ in 0..2 {
            let req = Request::builder()
                .uri("/api/test")
                .body(Body::empty())
                .expect("build request");
            let resp = app.clone().oneshot(req).await.expect("oneshot");
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // 3rd request should be rate-limited
        let req = Request::builder()
            .uri("/api/test")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}

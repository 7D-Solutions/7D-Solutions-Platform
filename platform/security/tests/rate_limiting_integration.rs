//! Integration test for cross-module rate limiting via TieredRateLimiter.
//!
//! Acceptance criteria from bd-397ij (GAP-08):
//! 1. Exceeding the write tier limit returns HTTP 429.
//! 2. The 429 response includes a `Retry-After` header.
//! 3. Composite key: exhausting tenant A does not affect tenant B.
//!
//! "Tenant A" and "tenant B" are simulated via distinct `X-Forwarded-For`
//! IPs.  When no JWT claims are present, the middleware uses IP as the
//! tenant ID, so different IPs produce independent Composite buckets —
//! exactly the isolation property we need to verify.
//!
//! Runs against real in-process Axum HTTP server — no mocks, no stubs.
//!
//! Tests are in a module named `rate_limiting_integration` so they match
//! the `cargo test -p security rate_limiting_integration` filter.

use axum::{body::Body, routing::post, Extension, Router};
use http::{Request, StatusCode};
use security::middleware::tiered_rate_limit_middleware;
use security::ratelimit::{RateLimitConfig, RateLimitKeyStrategy, TierDef, TieredRateLimiter};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

// ── helpers ───────────────────────────────────────────────────────────────────

fn write_read_limiter(write_limit: u32, read_limit: u32) -> Arc<TieredRateLimiter> {
    Arc::new(TieredRateLimiter::from_defs(vec![
        TierDef {
            name: "write".into(),
            config: RateLimitConfig::new(write_limit, Duration::from_secs(60)),
            routes: vec!["/api/".into()],
            strategy: RateLimitKeyStrategy::Composite,
            methods: Some(vec![
                "POST".into(),
                "PUT".into(),
                "PATCH".into(),
                "DELETE".into(),
            ]),
        },
        TierDef {
            name: "read".into(),
            config: RateLimitConfig::new(read_limit, Duration::from_secs(60)),
            routes: vec!["/api/".into()],
            strategy: RateLimitKeyStrategy::Composite,
            methods: Some(vec!["GET".into()]),
        },
    ]))
}

fn app_with_limiter(limiter: Arc<TieredRateLimiter>) -> Router {
    Router::new()
        .route("/api/bills", post(|| async { "created" }))
        .route("/api/bills", axum::routing::get(|| async { "listed" }))
        .layer(axum::middleware::from_fn(tiered_rate_limit_middleware))
        .layer(Extension(limiter))
}

fn post_from(ip: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/bills")
        .header("x-forwarded-for", ip)
        .body(Body::empty())
        .expect("build request")
}

fn get_from(ip: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/api/bills")
        .header("x-forwarded-for", ip)
        .body(Body::empty())
        .expect("build request")
}

// ── Tests — named under `rate_limiting_integration` for cargo filter match ────

mod rate_limiting_integration {
    use super::*;

    /// Exceeding the write tier limit must return HTTP 429.
    #[tokio::test]
    async fn write_tier_limit_returns_429() {
        let limiter = write_read_limiter(3, 100);
        let app = app_with_limiter(limiter);

        for i in 0..3 {
            let resp = app.clone().oneshot(post_from("10.0.0.1")).await.expect("request");
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "POST #{i} should be allowed within write quota"
            );
        }

        let resp = app.oneshot(post_from("10.0.0.1")).await.expect("request");
        assert_eq!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "POST beyond write quota must return 429"
        );
    }

    /// The 429 response must include a `Retry-After` header with a positive value.
    #[tokio::test]
    async fn write_tier_429_includes_retry_after_header() {
        let limiter = write_read_limiter(2, 100);
        let app = app_with_limiter(limiter);

        for _ in 0..2 {
            let _ = app.clone().oneshot(post_from("10.0.0.2")).await;
        }

        let resp = app.oneshot(post_from("10.0.0.2")).await.expect("request");
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let retry_after = resp.headers().get("retry-after");
        assert!(
            retry_after.is_some(),
            "429 response must include Retry-After header"
        );

        let secs: u64 = retry_after
            .unwrap()
            .to_str()
            .expect("header is valid UTF-8")
            .parse()
            .expect("Retry-After must be a number");
        assert!(secs > 0, "Retry-After must be a positive number of seconds");
    }

    /// Exhausting tenant A's write quota must NOT affect tenant B.
    ///
    /// Each `(tenant_id, ip)` pair owns an independent Composite bucket.
    /// Different source IPs simulate different tenants here.
    #[tokio::test]
    async fn write_tier_composite_key_tenant_isolation() {
        let limiter = write_read_limiter(3, 100);
        let app = app_with_limiter(limiter);

        // Exhaust tenant_a (IP 10.1.0.1).
        for _ in 0..3 {
            let resp = app.clone().oneshot(post_from("10.1.0.1")).await.expect("request");
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let blocked = app.clone().oneshot(post_from("10.1.0.1")).await.expect("request");
        assert_eq!(
            blocked.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "tenant_a must be blocked after quota exhaustion"
        );

        // tenant_b (IP 10.1.0.2) must have its own independent quota.
        for i in 0..3 {
            let resp = app.clone().oneshot(post_from("10.1.0.2")).await.expect("request");
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "tenant_b POST #{i} must be unaffected by tenant_a's exhaustion"
            );
        }
    }

    /// Exhausting the write tier must not affect the read tier.
    #[tokio::test]
    async fn write_and_read_tiers_are_independent() {
        let limiter = write_read_limiter(2, 10);
        let app = app_with_limiter(limiter);

        for _ in 0..2 {
            let _ = app.clone().oneshot(post_from("10.2.0.1")).await;
        }
        let write_blocked = app
            .clone()
            .oneshot(post_from("10.2.0.1"))
            .await
            .expect("request");
        assert_eq!(write_blocked.status(), StatusCode::TOO_MANY_REQUESTS);

        for i in 0..3 {
            let resp = app.clone().oneshot(get_from("10.2.0.1")).await.expect("request");
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "GET #{i} must succeed even though write tier is exhausted"
            );
        }
    }

    /// The auth tier (IpOnly) must share a single bucket across all tenant_ids
    /// on the same IP.
    #[tokio::test]
    async fn auth_tier_ip_only_shared_across_tenants() {
        use security::claims::{ActorType, VerifiedClaims};
        use uuid::Uuid;

        let limiter = Arc::new(TieredRateLimiter::from_defs(vec![TierDef {
            name: "auth".into(),
            config: RateLimitConfig::new(2, Duration::from_secs(60)),
            routes: vec!["/api/auth".into()],
            strategy: RateLimitKeyStrategy::IpOnly,
            methods: None,
        }]));

        let app = Router::new()
            .route("/api/auth/token", post(|| async { "token" }))
            .layer(axum::middleware::from_fn(tiered_rate_limit_middleware))
            .layer(Extension(limiter));

        let make_auth_req = |ip: &str, tenant_id: Uuid| {
            let claims = VerifiedClaims {
                user_id: Uuid::new_v4(),
                tenant_id,
                app_id: None,
                roles: vec![],
                perms: vec![],
                actor_type: ActorType::User,
                issued_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now(),
                token_id: Uuid::new_v4(),
                version: "1".into(),
            };
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/auth/token")
                .header("x-forwarded-for", ip)
                .body(Body::empty())
                .expect("build request");
            req.extensions_mut().insert(claims);
            req
        };

        let tenant_a = Uuid::new_v4();
        let tenant_b = Uuid::new_v4();

        // First request from tenant_a on 10.3.0.1 → OK.
        let r1 = app
            .clone()
            .oneshot(make_auth_req("10.3.0.1", tenant_a))
            .await
            .expect("r1");
        assert_eq!(r1.status(), StatusCode::OK);

        // Second request from tenant_b on the same IP → OK (second token consumed).
        let r2 = app
            .clone()
            .oneshot(make_auth_req("10.3.0.1", tenant_b))
            .await
            .expect("r2");
        assert_eq!(r2.status(), StatusCode::OK);

        // Third request — shared IP bucket exhausted regardless of tenant.
        let r3 = app
            .clone()
            .oneshot(make_auth_req("10.3.0.1", tenant_a))
            .await
            .expect("r3");
        assert_eq!(
            r3.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "IpOnly auth tier: third request from same IP must be blocked"
        );
    }
}

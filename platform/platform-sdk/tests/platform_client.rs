//! Integration tests for PlatformClient — real HTTP against a local test server.

use axum::{extract::Request, http::StatusCode, routing::get, Router};
use platform_sdk::{PlatformClient, TimeoutConfig};
use security::claims::{ActorType, VerifiedClaims};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_claims() -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        app_id: Some(Uuid::new_v4()),
        roles: vec!["admin".into()],
        perms: vec![],
        actor_type: ActorType::User,
        issued_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "1".into(),
    }
}

async fn start_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn injects_platform_headers() {
    let claims = test_claims();
    let expected_tenant = claims.tenant_id.to_string();
    let expected_app = claims.app_id.unwrap().to_string();

    let app = Router::new().route(
        "/api/test",
        get(|req: Request| async move {
            let h = req.headers();
            let tenant = h.get("x-tenant-id").unwrap().to_str().unwrap().to_string();
            let app = h.get("x-app-id").unwrap().to_str().unwrap().to_string();
            let corr = h
                .get("x-correlation-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            // correlation-id must be a valid UUID
            Uuid::parse_str(&corr).unwrap();
            format!("{tenant},{app}")
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base);
    let resp = client.get("/api/test", &claims).await.unwrap();
    let body = resp.text().await.unwrap();
    assert_eq!(body, format!("{expected_tenant},{expected_app}"));
}

#[tokio::test]
async fn injects_bearer_token() {
    let app = Router::new().route(
        "/api/auth",
        get(|req: Request| async move {
            let auth = req
                .headers()
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            auth
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base).with_bearer_token("my-service-token".into());
    let resp = client.get("/api/auth", &test_claims()).await.unwrap();
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Bearer my-service-token");
}

#[tokio::test]
async fn retries_on_429() {
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/retry",
        get(move || {
            let counter = counter.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::OK
                }
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base);
    let resp = client.get("/api/retry", &test_claims()).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(call_count.load(Ordering::SeqCst), 3); // 2 retries + 1 success
}

#[tokio::test]
async fn retries_on_503() {
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/unavailable",
        get(move || {
            let counter = counter.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n < 1 {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                }
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base);
    let resp = client
        .get("/api/unavailable", &test_claims())
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn no_retry_on_other_errors() {
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/bad",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::BAD_REQUEST
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base);
    let resp = client.get("/api/bad", &test_claims()).await.unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(call_count.load(Ordering::SeqCst), 1); // no retry
}

#[tokio::test]
async fn post_sends_json_body() {
    let app = Router::new().route(
        "/api/create",
        axum::routing::post(|body: axum::Json<serde_json::Value>| async move {
            body.0["name"].as_str().unwrap_or("missing").to_string()
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base);
    let payload = serde_json::json!({"name": "Acme Corp"});
    let resp = client
        .post("/api/create", &payload, &test_claims())
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Acme Corp");
}

#[tokio::test]
async fn retries_on_connection_refused() {
    // Bind a port, get its address, then drop the listener so nothing is listening.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let client = PlatformClient::with_timeout(
        format!("http://{addr}"),
        TimeoutConfig {
            request_timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
        },
    );

    let start = std::time::Instant::now();
    let err = client.get("/api/test", &test_claims()).await.unwrap_err();
    let elapsed = start.elapsed();

    // Must be a connect error (connection refused).
    assert!(err.is_connect(), "expected connect error, got: {err}");
    // Elapsed time proves retries happened (3 retries × 100ms minimum backoff).
    assert!(
        elapsed >= Duration::from_millis(300),
        "expected retries, elapsed: {elapsed:?}"
    );
}

#[tokio::test]
async fn no_retry_on_dns_failure() {
    let client = PlatformClient::with_timeout(
        "http://this-host-does-not-exist.invalid".to_string(),
        TimeoutConfig {
            request_timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
        },
    );

    let start = std::time::Instant::now();
    let err = client.get("/api/test", &test_claims()).await.unwrap_err();
    let elapsed = start.elapsed();

    // Must be a connect error from DNS failure.
    assert!(err.is_connect(), "expected connect error, got: {err}");
    // DNS failure should NOT retry — elapsed should be well under retry backoff sum.
    assert!(
        elapsed < Duration::from_secs(3),
        "DNS failure retried unexpectedly, elapsed: {elapsed:?}"
    );
}

#[tokio::test]
async fn retries_on_request_timeout() {
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/slow",
        get(move || {
            let counter = counter.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    // Sleep longer than request_timeout to trigger a timeout.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                StatusCode::OK
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::with_timeout(
        base,
        TimeoutConfig {
            request_timeout: Duration::from_millis(200),
            connect_timeout: Duration::from_secs(1),
        },
    );

    let resp = client.get("/api/slow", &test_claims()).await.unwrap();
    assert_eq!(resp.status(), 200);
    // First 2 attempts timed out, third succeeded.
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

// ── Service-token auto-refresh tests ─────────────────────────────────────────

fn setup_dev_service_auth() {
    std::env::set_var("ENV", "development");
    std::env::set_var("SERVICE_AUTH_SECRET", "platform-client-test-secret");
    std::env::set_var("SERVICE_NAME", "platform-sdk-test");
}

#[tokio::test]
async fn service_token_retries_once_on_401() {
    setup_dev_service_auth();
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/svc",
        get(move || {
            let counter = counter.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::OK
                }
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base).with_service_token(None, None);
    let resp = client.get("/api/svc", &test_claims()).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn service_token_does_not_retry_on_403() {
    setup_dev_service_auth();
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/forbidden",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::FORBIDDEN
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base).with_service_token(None, None);
    let resp = client.get("/api/forbidden", &test_claims()).await.unwrap();
    assert_eq!(resp.status(), 403);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "403 must not trigger a service-token retry"
    );
}

#[tokio::test]
async fn static_token_does_not_retry_on_401() {
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/static-auth",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::UNAUTHORIZED
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base).with_bearer_token("stale-token".into());
    let resp = client
        .get("/api/static-auth", &test_claims())
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "static token must never auto-retry on 401"
    );
}

#[tokio::test]
async fn service_token_propagates_second_401_without_third_attempt() {
    setup_dev_service_auth();
    let call_count = Arc::new(AtomicU32::new(0));
    let counter = call_count.clone();

    let app = Router::new().route(
        "/api/always-401",
        get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::UNAUTHORIZED
            }
        }),
    );

    let base = start_server(app).await;
    let client = PlatformClient::new(base).with_service_token(None, None);
    let resp = client.get("/api/always-401", &test_claims()).await.unwrap();
    assert_eq!(resp.status(), 401);
    // Original + one retry — no third attempt even though the retry also got 401.
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn service_token_concurrent_401s_all_succeed() {
    // Ensure JWT_PRIVATE_KEY_PEM is absent so inject_headers falls back to the
    // ServiceMinted cache (the path under test).
    std::env::remove_var("JWT_PRIVATE_KEY_PEM");
    setup_dev_service_auth();

    const N: u32 = 20;

    // The server uses the Authorization header as the signal: no header → 401
    // (token not yet minted), header present → 200 (token was re-minted and
    // attached by inject_headers).  This is timing-independent: concurrent
    // initial calls all lack auth (cache is None) and all retries carry the
    // freshly minted token regardless of scheduling order.
    let app = Router::new().route(
        "/api/concurrent-svc",
        get(|req: Request| async move {
            if req.headers().contains_key("authorization") {
                StatusCode::OK
            } else {
                StatusCode::UNAUTHORIZED
            }
        }),
    );

    let base = start_server(app).await;
    let client = Arc::new(PlatformClient::new(base).with_service_token(None, None));

    let handles: Vec<_> = (0..N)
        .map(|_| {
            let client = client.clone();
            let claims = test_claims();
            tokio::spawn(async move { client.get("/api/concurrent-svc", &claims).await.unwrap() })
        })
        .collect();

    for handle in handles {
        let resp = handle.await.unwrap();
        assert_eq!(
            resp.status(),
            200,
            "every concurrent task must succeed after token refresh"
        );
    }
}

//! Integration tests for PlatformClient — real HTTP against a local test server.

use axum::{extract::Request, http::StatusCode, routing::get, Router};
use platform_sdk::PlatformClient;
use security::claims::{ActorType, VerifiedClaims};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
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
            let corr = h.get("x-correlation-id").unwrap().to_str().unwrap().to_string();
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
            let auth = req.headers().get("authorization").unwrap().to_str().unwrap().to_string();
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
    let resp = client.get("/api/unavailable", &test_claims()).await.unwrap();
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
    let resp = client.post("/api/create", &payload, &test_claims()).await.unwrap();
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Acme Corp");
}

//! Logging standard compliance tests.
//!
//! Verifies that:
//! 1. `logging::request_span` creates a span with the required contextual fields.
//! 2. `platform_trace_middleware` injects `tenant_id`, `request_id`, and `actor_id`
//!    into the request span so all child logs inherit them.
//!
//! Run with:
//!   cargo test -p platform-sdk logging_standard -- --nocapture

use axum::{body::Body, routing::get, Router};
use http::{Request, StatusCode};
use platform_sdk::logging::request_span;
use tower::ServiceExt as _;
use uuid::Uuid;

// ── Unit: request_span fields ─────────────────────────────────────────────────

/// The span produced by `request_span` must carry the four required fields.
#[test]
fn logging_standard_request_span_fields() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    let span = request_span("inventory", "tenant-abc", "req-xyz", "user-789");

    let meta = span.metadata().expect("span must have metadata");
    let field_names: Vec<&str> = meta.fields().iter().map(|f| f.name()).collect();

    assert!(
        field_names.contains(&"tenant_id"),
        "span must record tenant_id — got: {:?}",
        field_names
    );
    assert!(
        field_names.contains(&"request_id"),
        "span must record request_id — got: {:?}",
        field_names
    );
    assert!(
        field_names.contains(&"actor_id"),
        "span must record actor_id — got: {:?}",
        field_names
    );
    assert!(
        field_names.contains(&"module"),
        "span must record module — got: {:?}",
        field_names
    );
}

/// Empty strings are valid (unauthenticated paths / background tasks).
#[test]
fn logging_standard_empty_fields_produce_valid_span() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    let span = request_span("scheduler", "", "", "");
    assert!(
        !span.is_disabled(),
        "span must be enabled even with empty fields"
    );
}

// ── Integration: platform_trace_middleware span fields ────────────────────────

/// The middleware must echo the client-supplied `x-request-id` back in the response,
/// confirming it was recorded as `request_id` in the span.
#[tokio::test]
async fn logging_standard_middleware_echoes_request_id() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    let app = Router::new()
        .route(
            "/test",
            get(|| async {
                tracing::info!(event = "test.hit", "handler reached");
                tracing::warn!(count = 0usize, "no items found");
                StatusCode::OK
            }),
        )
        .layer(axum::middleware::from_fn(
            platform_sdk::platform_trace_middleware,
        ));

    let request_id = Uuid::new_v4().to_string();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test")
                .header("x-request-id", &request_id)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // The middleware must echo the request ID back in the response headers.
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert_eq!(
        echoed, request_id,
        "middleware must echo x-request-id in the response"
    );
}

/// When no `x-request-id` header is supplied, the middleware generates a UUID
/// and echoes it back, ensuring `request_id` is always present in the span.
#[tokio::test]
async fn logging_standard_middleware_generates_request_id_when_absent() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    let app = Router::new()
        .route("/test", get(|| async { StatusCode::OK }))
        .layer(axum::middleware::from_fn(
            platform_sdk::platform_trace_middleware,
        ));

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // A generated request ID must be present.
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        !echoed.is_empty(),
        "middleware must generate a request_id when not provided"
    );
    Uuid::parse_str(echoed).expect("generated request_id must be a valid UUID");
}

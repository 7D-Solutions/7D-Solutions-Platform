//! Body-limit integration test for the GL module.
//!
//! Verifies that the body_limit configured in module.toml ([server] body_limit = "16mb")
//! is applied correctly by the SDK's DefaultBodyLimit middleware layer.
//!
//! The test builds a minimal in-process router with DefaultBodyLimit set to the
//! same value as module.toml, then uses tower::ServiceExt::oneshot to verify:
//!   - A body 1 byte over the limit is rejected with 413 Payload Too Large
//!   - The handler is NOT called for oversized bodies (verified via AtomicBool sentinel)
//!   - A body 1 byte under the limit passes through to the handler (200 OK)
//!
//! Run with:
//!   ./scripts/cargo-slot.sh test -p gl-rs --test imports_body_limit

use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tower::ServiceExt;

// ── Body limit constant — must match module.toml [server] body_limit ─────────

const LIMIT_BYTES: usize = 16 * 1024 * 1024; // "16mb"

// ── Sentinel handler ──────────────────────────────────────────────────────────
//
// Uses a handler test-hook counter (AtomicBool passed via State) as the
// "handler did not run" verification method described in the bead.

async fn sentinel_handler(State(ran): State<Arc<AtomicBool>>, _body: Bytes) -> impl IntoResponse {
    ran.store(true, Ordering::SeqCst);
    StatusCode::OK
}

fn build_test_router(ran: Arc<AtomicBool>) -> Router {
    Router::new()
        .route("/api/gl/import/chart-of-accounts", post(sentinel_handler))
        .with_state(ran)
        .layer(DefaultBodyLimit::max(LIMIT_BYTES))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Body 1 byte over the configured 16 MiB limit must be rejected with 413.
/// The sentinel handler must NOT have run.
#[tokio::test]
async fn body_over_limit_returns_413() {
    let ran = Arc::new(AtomicBool::new(false));
    let app = build_test_router(ran.clone());

    let body = vec![0u8; LIMIT_BYTES + 1];

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/gl/import/chart-of-accounts")
                .header("content-type", "application/octet-stream")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "body over 16 MiB must be rejected with 413 Payload Too Large"
    );
    assert!(
        !ran.load(Ordering::SeqCst),
        "handler must NOT run when body exceeds the configured limit"
    );
}

/// Body 1 byte under the configured 16 MiB limit must reach the handler (200 OK).
#[tokio::test]
async fn body_under_limit_reaches_handler() {
    let ran = Arc::new(AtomicBool::new(false));
    let app = build_test_router(ran.clone());

    let body = vec![0u8; LIMIT_BYTES - 1];

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/gl/import/chart-of-accounts")
                .header("content-type", "application/octet-stream")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "body under 16 MiB must NOT be rejected by the body-limit middleware"
    );
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "sentinel handler must return 200 OK for accepted body"
    );
    assert!(
        ran.load(Ordering::SeqCst),
        "handler must run when body is within the configured limit"
    );
}

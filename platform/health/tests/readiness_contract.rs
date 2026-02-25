//! Integration tests for the health crate's readiness probe contract.
//!
//! Validates JSON shapes, HTTP status codes, and field semantics
//! against docs/HEALTH-CONTRACT.md.

use axum::{routing::get, Router};
use health::{
    build_ready_response, db_check, healthz, nats_check, ready_response_to_axum, CheckStatus,
    ReadyStatus,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ready_route_all_up() -> Router {
    Router::new().route(
        "/api/ready",
        get(|| async {
            let checks = vec![db_check(3, None), nats_check(true, 1)];
            let resp = build_ready_response("test-svc", "0.1.0", checks);
            ready_response_to_axum(resp)
        }),
    )
}

fn ready_route_db_down() -> Router {
    Router::new().route(
        "/api/ready",
        get(|| async {
            let checks = vec![
                db_check(0, Some("connection refused".into())),
                nats_check(true, 2),
            ];
            let resp = build_ready_response("test-svc", "0.1.0", checks);
            ready_response_to_axum(resp)
        }),
    )
}

fn liveness_route() -> Router {
    Router::new().route("/healthz", get(healthz))
}

async fn get_json(app: Router, uri: &str) -> (u16, serde_json::Value) {
    let req = axum::http::Request::builder()
        .uri(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

// ---------------------------------------------------------------------------
// Liveness probe tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn healthz_returns_200() {
    let (status, _) = get_json(liveness_route(), "/healthz").await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn healthz_body_matches_contract() {
    let (_, json) = get_json(liveness_route(), "/healthz").await;
    assert_eq!(json["status"], "alive", "liveness body must be {{\"status\":\"alive\"}}");
    // Contract says the body is exactly {"status":"alive"} — no extra fields.
    let obj = json.as_object().unwrap();
    assert_eq!(obj.len(), 1, "liveness response must have exactly one field");
}

// ---------------------------------------------------------------------------
// Readiness probe — all checks up (HTTP 200, status=ready)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ready_all_up_returns_200() {
    let (status, _) = get_json(ready_route_all_up(), "/api/ready").await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn ready_all_up_json_shape_matches_contract() {
    let (_, json) = get_json(ready_route_all_up(), "/api/ready").await;

    // Required top-level fields per HEALTH-CONTRACT.md
    assert_eq!(json["service_name"], "test-svc");
    assert_eq!(json["version"], "0.1.0");
    assert_eq!(json["status"], "ready");
    assert_eq!(json["degraded"], false);
    assert!(json["timestamp"].is_string(), "timestamp must be a string");

    // checks array
    let checks = json["checks"].as_array().expect("checks must be an array");
    assert_eq!(checks.len(), 2);

    for check in checks {
        assert!(check["name"].is_string(), "check.name must be a string");
        assert!(
            check["status"].as_str() == Some("up") || check["status"].as_str() == Some("down"),
            "check.status must be 'up' or 'down'"
        );
        assert!(check["latency_ms"].is_u64(), "check.latency_ms must be u64");
    }
}

#[tokio::test]
async fn ready_all_up_no_error_fields() {
    let (_, json) = get_json(ready_route_all_up(), "/api/ready").await;
    let checks = json["checks"].as_array().unwrap();
    for check in checks {
        assert!(
            check.get("error").is_none(),
            "error field must be absent when check is up"
        );
    }
}

#[tokio::test]
async fn ready_all_up_timestamp_is_iso8601() {
    let (_, json) = get_json(ready_route_all_up(), "/api/ready").await;
    let ts = json["timestamp"].as_str().unwrap();
    // chrono can parse it back — confirms ISO 8601 / RFC 3339
    chrono::DateTime::parse_from_rfc3339(ts)
        .expect("timestamp must be valid RFC 3339 / ISO 8601");
}

// ---------------------------------------------------------------------------
// Readiness probe — dependency down (HTTP 503, status=down)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ready_db_down_returns_503() {
    let (status, _) = get_json(ready_route_db_down(), "/api/ready").await;
    assert_eq!(status, 503);
}

#[tokio::test]
async fn ready_db_down_status_is_down() {
    let (_, json) = get_json(ready_route_db_down(), "/api/ready").await;
    assert_eq!(json["status"], "down");
    assert_eq!(json["degraded"], false);
}

#[tokio::test]
async fn ready_db_down_error_field_present() {
    let (_, json) = get_json(ready_route_db_down(), "/api/ready").await;
    let checks = json["checks"].as_array().unwrap();
    let db = checks.iter().find(|c| c["name"] == "database").unwrap();
    assert_eq!(db["status"], "down");
    assert!(
        db.get("error").is_some(),
        "error field must be present when check is down"
    );
    assert_eq!(db["error"], "connection refused");
}

// ---------------------------------------------------------------------------
// Helper function unit-level tests (via public API)
// ---------------------------------------------------------------------------

#[test]
fn db_check_up_has_no_error() {
    let c = db_check(5, None);
    assert_eq!(c.name, "database");
    assert_eq!(c.status, CheckStatus::Up);
    assert!(c.error.is_none());
}

#[test]
fn db_check_down_has_error() {
    let c = db_check(0, Some("timeout".into()));
    assert_eq!(c.status, CheckStatus::Down);
    assert_eq!(c.error.as_deref(), Some("timeout"));
}

#[test]
fn nats_check_connected() {
    let c = nats_check(true, 2);
    assert_eq!(c.name, "nats");
    assert_eq!(c.status, CheckStatus::Up);
    assert!(c.error.is_none());
}

#[test]
fn nats_check_disconnected() {
    let c = nats_check(false, 0);
    assert_eq!(c.status, CheckStatus::Down);
    assert!(c.error.is_some());
}

#[test]
fn build_ready_response_mixed_checks() {
    let checks = vec![db_check(3, None), nats_check(false, 0)];
    let resp = build_ready_response("svc", "1.0.0", checks);
    // Any down → status=down per contract
    assert_eq!(resp.status, ReadyStatus::Down);
    assert!(!resp.degraded);
    assert_eq!(resp.checks.len(), 2);
}

#[test]
fn ready_response_to_axum_returns_ok_for_ready() {
    let checks = vec![db_check(1, None)];
    let resp = build_ready_response("s", "0.1.0", checks);
    assert!(ready_response_to_axum(resp).is_ok());
}

#[test]
fn ready_response_to_axum_returns_err_for_down() {
    let checks = vec![db_check(0, Some("fail".into()))];
    let resp = build_ready_response("s", "0.1.0", checks);
    assert!(ready_response_to_axum(resp).is_err());
}

//! Integration tests for POST /api/reporting/rebuild admin endpoint (bd-22lo).

mod helpers;

use axum::{body::Body, http::Request};
use helpers::{body_json, build_test_app, setup_db, unique_tenant};
use serial_test::serial;
use tower::ServiceExt;

// ── Auth gate ───────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn rebuild_without_admin_token_returns_403() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    std::env::set_var("ADMIN_TOKEN", "test-secret");

    let body = serde_json::json!({
        "tenant_id": tid.to_string(),
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/reporting/rebuild")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
}

#[tokio::test]
#[serial]
async fn rebuild_wrong_token_returns_403() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    std::env::set_var("ADMIN_TOKEN", "correct-token");

    let body = serde_json::json!({
        "tenant_id": tid.to_string(),
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/reporting/rebuild")
                .header("content-type", "application/json")
                .header("x-admin-token", "wrong-token")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
}

#[tokio::test]
#[serial]
async fn rebuild_with_valid_token_returns_200() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    std::env::set_var("ADMIN_TOKEN", "test-rebuild-token");

    let body = serde_json::json!({
        "tenant_id": tid.to_string(),
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/reporting/rebuild")
                .header("content-type", "application/json")
                .header("x-admin-token", "test-rebuild-token")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    assert!(json["rows_upserted"].is_number());
}

// ── Validation ──────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn rebuild_invalid_date_range_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    std::env::set_var("ADMIN_TOKEN", "test-rebuild-token");

    let body = serde_json::json!({
        "tenant_id": tid.to_string(),
        "from": "2026-02-01",
        "to": "2026-01-01"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/reporting/rebuild")
                .header("content-type", "application/json")
                .header("x-admin-token", "test-rebuild-token")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "validation_error");
}

#[tokio::test]
#[serial]
async fn rebuild_empty_tenant_returns_400() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    std::env::set_var("ADMIN_TOKEN", "test-rebuild-token");

    let body = serde_json::json!({
        "tenant_id": "",
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/reporting/rebuild")
                .header("content-type", "application/json")
                .header("x-admin-token", "test-rebuild-token")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let json = body_json(resp).await;
    assert_eq!(json["error"], "validation_error");
}

#[tokio::test]
#[serial]
async fn rebuild_no_admin_token_env_returns_403() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    // Clear ADMIN_TOKEN so rebuild is disabled
    std::env::remove_var("ADMIN_TOKEN");

    let body = serde_json::json!({
        "tenant_id": tid.to_string(),
        "from": "2026-01-01",
        "to": "2026-01-31"
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/reporting/rebuild")
                .header("content-type", "application/json")
                .header("x-admin-token", "anything")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
}

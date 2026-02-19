//! E2E Test: Inbound webhook → idempotent ingest → routed event observed (bd-1r5b)
//!
//! **Coverage:**
//! 1. POST /api/webhooks/inbound/internal with a novel idempotency key.
//!    - Response: 200 OK, status=accepted, ingest_id > 0.
//!    - DB: one row in integrations_webhook_ingest, processed_at set.
//!    - Outbox: webhook.received + webhook.routed events exactly once.
//! 2. Replay identical request (same idempotency key).
//!    - Response: 200 OK, status=duplicate, same ingest_id.
//!    - DB: still exactly one row.
//!    - Outbox: count unchanged (no double-routing).
//! 3. Unsupported system → 404.
//!
//! **Pattern:** In-process Axum router + real integrations-postgres (port 5449).
//! No Docker spin-up, no mocks, no stubs.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::get_integrations_pool;
use integrations_rs::{http, metrics::IntegrationsMetrics, AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Run integrations migrations against the pool.
async fn run_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/integrations/db/migrations")
        .run(pool)
        .await
        .expect("integrations migrations failed");
}

/// Build an in-process integrations router wired to the real pool.
fn make_router(pool: PgPool) -> axum::Router {
    let m = Arc::new(IntegrationsMetrics::new().expect("metrics init failed"));
    let state = Arc::new(AppState { pool, metrics: m });
    http::router(state)
}

/// POST /api/webhooks/inbound/{system} and return (status, body).
async fn post_webhook(
    router: &axum::Router,
    system: &str,
    idempotency_key: &str,
    event_type: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let body_str = payload.to_string();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/webhooks/inbound/{}", system))
        .header("content-type", "application/json")
        .header("x-webhook-id", idempotency_key)
        .body(Body::from(body_str))
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
    (status, body)
}

/// Delete all test rows scoped to the given app_id from both ingest + outbox.
async fn cleanup(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Happy path: inbound internal webhook accepted, persisted, and routed.
#[tokio::test]
async fn test_webhook_ingest_accepted_and_routed() {
    let pool = get_integrations_pool().await;
    run_migrations(&pool).await;

    let app_id = format!("e2e-wh-{}", Uuid::new_v4().simple());
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());
    let idem_key = format!("evt-{}", Uuid::new_v4().simple());
    let event_type = "order.fulfilled";

    let payload = json!({
        "event_type": event_type,
        "order_id": "ord-abc-123",
        "amount_minor": 4999
    });

    // ── First delivery ───────────────────────────────────────────────────
    // We need to inject the app_id into the request. The webhook handler reads
    // app_id from the X-App-Id header.
    let body_str = payload.to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/api/webhooks/inbound/internal")
        .header("content-type", "application/json")
        .header("x-webhook-id", &idem_key)
        .header("x-app-id", &app_id)
        .body(Body::from(body_str))
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));

    assert_eq!(status, StatusCode::OK, "expected 200; body={}", body);
    assert_eq!(body["status"], "accepted", "body={}", body);
    let ingest_id = body["ingest_id"].as_i64().expect("ingest_id must be integer");
    assert!(ingest_id > 0, "ingest_id must be positive");

    // ── DB: verify ingest row exists and is marked processed ─────────────
    let row: Option<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT system, event_type, processed_at IS NOT NULL
         FROM integrations_webhook_ingest
         WHERE id = $1 AND app_id = $2",
    )
    .bind(ingest_id)
    .bind(&app_id)
    .fetch_optional(&pool)
    .await
    .expect("ingest query failed");

    let (system, db_event_type, is_processed) = row.expect("ingest row must exist");
    assert_eq!(system, "internal");
    assert_eq!(db_event_type.as_deref(), Some(event_type));
    assert!(is_processed, "processed_at must be set after ingest");

    // ── Outbox: webhook.received must exist ──────────────────────────────
    let received_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND event_type = 'webhook.received'
           AND aggregate_type = 'webhook' AND aggregate_id = $2",
    )
    .bind(&app_id)
    .bind(ingest_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(received_count.0, 1, "webhook.received must appear exactly once");

    // ── Outbox: webhook.routed must exist (internal passthrough) ─────────
    let routed_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND event_type = 'webhook.routed'
           AND aggregate_type = 'webhook' AND aggregate_id = $2",
    )
    .bind(&app_id)
    .bind(ingest_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox routed query failed");
    assert_eq!(routed_count.0, 1, "webhook.routed must appear exactly once");

    cleanup(&pool, &app_id).await;
}

/// Idempotency: replaying the same webhook must not double-route.
#[tokio::test]
async fn test_webhook_replay_does_not_double_route() {
    let pool = get_integrations_pool().await;
    run_migrations(&pool).await;

    let app_id = format!("e2e-wh-dedup-{}", Uuid::new_v4().simple());
    cleanup(&pool, &app_id).await;

    let router = make_router(pool.clone());
    let idem_key = format!("evt-dedup-{}", Uuid::new_v4().simple());
    let payload_str = json!({
        "event_type": "inventory.adjusted",
        "sku": "SKU-001",
        "delta": -5
    })
    .to_string();

    // Helper: send POST with given app_id header
    let send = |key: &str| {
        let router = router.clone();
        let body = payload_str.clone();
        let app = app_id.clone();
        let k = key.to_string();
        async move {
            let request = Request::builder()
                .method("POST")
                .uri("/api/webhooks/inbound/internal")
                .header("content-type", "application/json")
                .header("x-webhook-id", &k)
                .header("x-app-id", &app)
                .body(Body::from(body))
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
            (status, body)
        }
    };

    // ── First delivery ───────────────────────────────────────────────────
    let (s1, b1) = send(&idem_key).await;
    assert_eq!(s1, StatusCode::OK, "first: {}", b1);
    assert_eq!(b1["status"], "accepted", "first: {}", b1);
    let ingest_id = b1["ingest_id"].as_i64().expect("ingest_id");

    // Capture outbox count after first ingest
    let outbox_after_first: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_id = $2",
    )
    .bind(&app_id)
    .bind(ingest_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    let first_outbox_count = outbox_after_first.0;
    assert!(first_outbox_count >= 1, "at least webhook.received expected");

    // ── Replay identical delivery ─────────────────────────────────────────
    let (s2, b2) = send(&idem_key).await;
    assert_eq!(s2, StatusCode::OK, "replay: {}", b2);
    assert_eq!(b2["status"], "duplicate", "replay must be flagged as duplicate; body={}", b2);
    let replay_ingest_id = b2["ingest_id"].as_i64().expect("replay ingest_id");
    assert_eq!(
        ingest_id, replay_ingest_id,
        "replay must return the original ingest_id"
    );

    // ── DB: exactly one row ───────────────────────────────────────────────
    let row_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_webhook_ingest
         WHERE app_id = $1 AND idempotency_key = $2",
    )
    .bind(&app_id)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("row count");
    assert_eq!(row_count.0, 1, "must have exactly one ingest row after replay");

    // ── Outbox: count unchanged after replay ──────────────────────────────
    let outbox_after_replay: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_id = $2",
    )
    .bind(&app_id)
    .bind(ingest_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox count after replay");
    assert_eq!(
        outbox_after_replay.0, first_outbox_count,
        "outbox count must not increase on replay"
    );

    cleanup(&pool, &app_id).await;
}

/// Unsupported system name returns 404.
#[tokio::test]
async fn test_unsupported_system_rejected() {
    let pool = get_integrations_pool().await;
    run_migrations(&pool).await;

    let router = make_router(pool.clone());
    let (status, body) = post_webhook(
        &router,
        "acme-payments",
        "evt-noop",
        "irrelevant",
        json!({ "event_type": "irrelevant" }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown system must return 404; body={}",
        body
    );
}

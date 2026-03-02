/// E2E tests for the control-plane tenant provisioning API
///
/// Verifies:
/// 1. POST /api/control/tenants creates tenant record in 'pending' state
/// 2. Idempotency key prevents duplicate creates (returns 200 on replay)
/// 3. Guard→Mutation→Outbox atomicity: both tenant row and outbox event exist
/// 4. Explicit tenant_id supplied by caller is honoured
/// 5. Duplicate tenant_id returns 409 Conflict
/// 6. Empty idempotency_key is rejected with 422
///
/// These tests use an in-process Axum router connected to the real
/// tenant-registry database (localhost:5441). No Docker needed.
mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::get_tenant_registry_pool;
use control_plane::routes::provisioning_router;
use control_plane::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Test Setup
// ============================================================================

/// Ensure the control-plane tables exist (migration idempotent).
///
/// The smoke test resets `tenants` + `provisioning_steps` for its own tests;
/// we recreate the control-plane tables here if they were dropped.
async fn ensure_control_plane_tables(pool: &PgPool) {
    // Ensure status constraint includes 'pending' and 'failed'.
    // We recreate the tenants table constraint via a separate migration.
    // Since the smoke test may have reset the DB, apply our migration idempotently.
    let migration_sql = include_str!(
        "../../platform/tenant-registry/db/migrations/20260217000001_add_control_plane_tables.sql"
    );

    // Tables may already exist; the migration uses IF NOT EXISTS so it's safe.
    sqlx::raw_sql(migration_sql).execute(pool).await.ok(); // Ignore errors (e.g. tables already exist from a clean state)
}

/// Helper: build in-process provisioning router
async fn make_router(pool: PgPool) -> axum::Router {
    let state = Arc::new(AppState::new(pool));
    provisioning_router(state)
}

/// Helper: POST /api/control/tenants with given body; return (status, body)
async fn post_create_tenant(router: &axum::Router, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/control/tenants")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();

    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));

    (status, body)
}

/// Cleanup: remove test rows from provisioning_requests, provisioning_outbox, tenants
async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM provisioning_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM provisioning_requests WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_create_tenant_returns_pending_status() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let idempotency_key = Uuid::new_v4().to_string();
    let router = make_router(pool.clone()).await;

    let (status, body) = post_create_tenant(
        &router,
        json!({
            "idempotency_key": idempotency_key,
            "environment": "development"
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "Expected 202 Accepted, got: {}\nbody: {}",
        status,
        body
    );
    assert_eq!(body["status"], "pending", "Tenant status should be pending");
    assert_eq!(body["idempotency_key"], idempotency_key);
    assert!(
        body["tenant_id"].is_string(),
        "tenant_id should be a UUID string"
    );

    let tenant_id: Uuid = body["tenant_id"].as_str().unwrap().parse().unwrap();

    // Verify DB: tenant row exists with status=pending
    let db_status: String = sqlx::query_scalar("SELECT status FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&pool)
        .await
        .expect("Tenant should exist in DB");

    assert_eq!(db_status, "pending", "DB status should be pending");

    cleanup(&pool, tenant_id).await;

    println!("✅ test_create_tenant_returns_pending_status");
}

#[tokio::test]
async fn test_outbox_event_written_atomically() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let idempotency_key = Uuid::new_v4().to_string();
    let router = make_router(pool.clone()).await;

    let (status, body) = post_create_tenant(
        &router,
        json!({
            "idempotency_key": idempotency_key,
            "environment": "development"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let tenant_id: Uuid = body["tenant_id"].as_str().unwrap().parse().unwrap();

    // Verify outbox event was written atomically
    let event_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM provisioning_outbox
        WHERE tenant_id = $1 AND event_type = 'tenant.provisioning_started'
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Outbox query should succeed");

    assert_eq!(
        event_count, 1,
        "Exactly one tenant.provisioning_started event should exist in outbox"
    );

    // Verify outbox event is not yet published (published_at = null)
    let published_at: Option<String> = sqlx::query_scalar(
        "SELECT published_at::text FROM provisioning_outbox WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Outbox event should exist");

    assert!(
        published_at.is_none(),
        "Outbox event should not be published yet (published_at = null)"
    );

    cleanup(&pool, tenant_id).await;

    println!("✅ test_outbox_event_written_atomically");
}

#[tokio::test]
async fn test_idempotency_replay_returns_200() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let idempotency_key = Uuid::new_v4().to_string();
    let router = make_router(pool.clone()).await;

    let body = json!({
        "idempotency_key": idempotency_key,
        "environment": "development"
    });

    // First request: 202 Accepted
    let (status1, body1) = post_create_tenant(&router, body.clone()).await;
    assert_eq!(status1, StatusCode::ACCEPTED);
    let tenant_id: Uuid = body1["tenant_id"].as_str().unwrap().parse().unwrap();

    // Second request with same key: 200 OK (idempotency replay)
    let (status2, body2) = post_create_tenant(&router, body.clone()).await;
    assert_eq!(
        status2,
        StatusCode::OK,
        "Duplicate idempotency key should return 200 OK"
    );
    assert_eq!(
        body2["tenant_id"].as_str().unwrap(),
        body1["tenant_id"].as_str().unwrap(),
        "Replayed tenant_id should match original"
    );
    assert_eq!(body2["idempotency_key"], idempotency_key);

    // Only ONE tenant row and ONE outbox event should exist
    let tenant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tenant_count, 1, "Only one tenant row should exist");

    let outbox_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM provisioning_outbox WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(outbox_count, 1, "Only one outbox event should exist");

    cleanup(&pool, tenant_id).await;

    println!("✅ test_idempotency_replay_returns_200");
}

#[tokio::test]
async fn test_explicit_tenant_id_is_honoured() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let explicit_tenant_id = Uuid::new_v4();
    let idempotency_key = Uuid::new_v4().to_string();
    let router = make_router(pool.clone()).await;

    let (status, body) = post_create_tenant(
        &router,
        json!({
            "tenant_id": explicit_tenant_id,
            "idempotency_key": idempotency_key,
            "environment": "staging"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let returned_id: Uuid = body["tenant_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        returned_id, explicit_tenant_id,
        "Returned tenant_id should match explicitly supplied value"
    );

    cleanup(&pool, explicit_tenant_id).await;

    println!("✅ test_explicit_tenant_id_is_honoured");
}

#[tokio::test]
async fn test_duplicate_tenant_id_returns_409() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let tenant_id = Uuid::new_v4();
    let router = make_router(pool.clone()).await;

    // First request: succeeds
    let (status1, _) = post_create_tenant(
        &router,
        json!({
            "tenant_id": tenant_id,
            "idempotency_key": Uuid::new_v4().to_string(),
            "environment": "development"
        }),
    )
    .await;
    assert_eq!(status1, StatusCode::ACCEPTED);

    // Second request with same tenant_id but DIFFERENT idempotency key: 409 Conflict
    let (status2, body2) = post_create_tenant(
        &router,
        json!({
            "tenant_id": tenant_id,
            "idempotency_key": Uuid::new_v4().to_string(),
            "environment": "development"
        }),
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::CONFLICT,
        "Duplicate tenant_id should return 409 Conflict, got: {}\nbody: {}",
        status2,
        body2
    );

    cleanup(&pool, tenant_id).await;

    println!("✅ test_duplicate_tenant_id_returns_409");
}

#[tokio::test]
async fn test_empty_idempotency_key_returns_422() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let router = make_router(pool.clone()).await;

    let (status, body) = post_create_tenant(
        &router,
        json!({
            "idempotency_key": "",
            "environment": "development"
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "Empty idempotency_key should return 422, got: {}\nbody: {}",
        status,
        body
    );

    println!("✅ test_empty_idempotency_key_returns_422");
}

#[tokio::test]
async fn test_idempotency_key_stored_in_provisioning_requests() {
    let pool = get_tenant_registry_pool().await;
    ensure_control_plane_tables(&pool).await;

    let idempotency_key = Uuid::new_v4().to_string();
    let router = make_router(pool.clone()).await;

    let (status, body) = post_create_tenant(
        &router,
        json!({
            "idempotency_key": idempotency_key,
            "environment": "development"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    let tenant_id: Uuid = body["tenant_id"].as_str().unwrap().parse().unwrap();

    // Verify idempotency_key recorded in provisioning_requests
    let stored_tenant_id: Uuid = sqlx::query_scalar(
        "SELECT tenant_id FROM provisioning_requests WHERE idempotency_key = $1",
    )
    .bind(&idempotency_key)
    .fetch_one(&pool)
    .await
    .expect("Idempotency key should be stored in provisioning_requests");

    assert_eq!(
        stored_tenant_id, tenant_id,
        "Stored tenant_id should match returned tenant_id"
    );

    cleanup(&pool, tenant_id).await;

    println!("✅ test_idempotency_key_stored_in_provisioning_requests");
}

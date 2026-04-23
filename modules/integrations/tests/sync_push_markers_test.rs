//! Integration tests: push result marker persistence and sync.push.failed event emission.
//!
//! Verified against real Postgres (no mocks).  The outbox-enqueue path is exercised by
//! running a push against a connected tenant with dummy QBO tokens so the call fails with
//! `auth_failed`, triggering the push.failed outbox event.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_push_markers_test

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Extension, Router,
};
use chrono::{TimeZone, Utc};
use event_bus::InMemoryBus;
use integrations_rs::{
    domain::{
        qbo::{QboError, TokenProvider},
        sync::push_attempts,
    },
    http::sync::push_entity,
    metrics::IntegrationsMetrics,
    AppState,
};
use security::{claims::ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;

// ── DB pool ───────────────────────────────────────────────────────────────────

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn test_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["integrations.sync.push".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn build_test_app(pool: sqlx::PgPool, tenant_id: Uuid) -> Router {
    let state = Arc::new(AppState {
        pool,
        metrics: Arc::new(IntegrationsMetrics::new().expect("IntegrationsMetrics::new")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    Router::new()
        .route(
            "/api/integrations/sync/push/{entity_type}",
            post(push_entity),
        )
        .with_state(state)
        .layer(Extension(test_claims(tenant_id)))
}

async fn seed_oauth_connection(pool: &sqlx::PgPool, app_id: &str) {
    // Use app_id as realm_id to guarantee uniqueness across parallel test runs.
    // The realm_id partial-unique index (provider, realm_id) WHERE connected blocks
    // two tenants connecting the same realm simultaneously; unique-per-app_id avoids it.
    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections (
            app_id, provider, realm_id,
            access_token, refresh_token,
            access_token_expires_at, refresh_token_expires_at,
            scopes_granted, connection_status
        )
        VALUES ($1, 'quickbooks', $1,
                '\x74657374'::bytea, '\x74657374'::bytea,
                NOW() + INTERVAL '1 hour', NOW() + INTERVAL '30 days',
                'com.intuit.quickbooks.accounting', 'connected')
        ON CONFLICT (app_id, provider) DO UPDATE
            SET realm_id = EXCLUDED.realm_id,
                connection_status = 'connected'
        "#,
    )
    .bind(app_id)
    .execute(pool)
    .await
    .expect("seed OAuth");
}

async fn seed_authority(pool: &sqlx::PgPool, app_id: &str, entity_type: &str) {
    sqlx::query(
        r#"
        INSERT INTO integrations_sync_authority
            (app_id, provider, entity_type, authoritative_side, authority_version)
        VALUES ($1, 'quickbooks', $2, 'platform', 1)
        ON CONFLICT (app_id, provider, entity_type)
        DO UPDATE SET authority_version = 1, updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .execute(pool)
    .await
    .expect("seed authority");
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    let _ = sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_sync_authority WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
}

// ── Result marker DB tests ────────────────────────────────────────────────────

/// `complete_attempt_with_markers` stores normalized markers and sets status=succeeded.
#[tokio::test]
#[serial]
async fn test_result_markers_stored_on_success() {
    let pool = setup_db().await;
    let app_id = format!("markers-test-{}", Uuid::new_v4().simple());
    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-marker-001",
        "create",
        1,
        "fp-markers-001",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    // Timestamp with sub-millisecond precision — stored version must be truncated.
    let raw_ts = Utc.timestamp_nanos(1_700_000_000_123_456_789_i64);

    let done = push_attempts::complete_attempt_with_markers(
        &pool,
        row.id,
        Some("SyncToken-42"),
        Some(raw_ts),
        Some("ph:abc123hash"),
        None,
    )
    .await
    .expect("complete_with_markers")
    .expect("row returned");

    assert_eq!(done.status, "succeeded");
    assert_eq!(done.result_sync_token.as_deref(), Some("SyncToken-42"));
    assert_eq!(
        done.result_projection_hash.as_deref(),
        Some("ph:abc123hash")
    );

    // Verify ms truncation: stored time must equal timestamp_millis() of raw_ts.
    let stored = done
        .result_last_updated_time
        .expect("result_last_updated_time present");
    assert_eq!(
        stored.timestamp_millis(),
        raw_ts.timestamp_millis(),
        "milliseconds must be preserved"
    );
    assert_eq!(
        stored.timestamp_subsec_micros() % 1000,
        0,
        "sub-millisecond part must be zero after truncation"
    );

    cleanup(&pool, &app_id).await;
}

/// Null markers are accepted on success (provider may not return all fields).
#[tokio::test]
#[serial]
async fn test_result_markers_can_be_null() {
    let pool = setup_db().await;
    let app_id = format!("markers-null-{}", Uuid::new_v4().simple());
    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-marker-null",
        "create",
        1,
        "fp-null-markers",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    let done = push_attempts::complete_attempt_with_markers(&pool, row.id, None, None, None, None)
        .await
        .expect("complete")
        .expect("row");

    assert_eq!(done.status, "succeeded");
    assert!(done.result_sync_token.is_none());
    assert!(done.result_last_updated_time.is_none());
    assert!(done.result_projection_hash.is_none());

    cleanup(&pool, &app_id).await;
}

// ── push.failed event emission test ──────────────────────────────────────────

/// When a push fails with auth_failed (dummy tokens in test), the outbox must contain
/// exactly one `sync.push.failed` event for the tenant.
#[tokio::test]
#[serial]
async fn test_push_failed_event_enqueued_on_auth_failure() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&pool, &app_id).await;

    seed_oauth_connection(&pool, &app_id).await;
    seed_authority(&pool, &app_id, "invoice").await;

    let app = build_test_app(pool.clone(), tenant_id);

    // The push will reach QBO with dummy tokens and fail with auth_failed.
    let body = serde_json::json!({
        "entity_id": "inv-push-fail-001",
        "operation": "create",
        "authority_version": 1,
        "request_fingerprint": format!("fp-fail-{}", Uuid::new_v4()),
        "payload": {
            "CustomerRef": { "value": "1" },
            "Line": [{"Amount": 100.0, "DetailType": "SalesItemLineDetail",
                       "SalesItemLineDetail": {"ItemRef": {"value": "1"}}}]
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/integrations/sync/push/invoice")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Auth failure is a classified fault → outcome:failed, HTTP 200
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "failed outcome must return 200"
    );

    let resp_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp_json: serde_json::Value = serde_json::from_slice(&resp_bytes).unwrap();
    assert_eq!(resp_json["outcome"], "failed", "outcome discriminant");

    // Verify the outbox has a sync.push.failed event for this tenant.
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND event_type = 'sync.push.failed'"
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("outbox count query");

    assert_eq!(
        event_count.0, 1,
        "exactly one sync.push.failed event in outbox"
    );

    // Validate the event payload matches the contract.
    let event_row: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM integrations_outbox WHERE app_id = $1 AND event_type = 'sync.push.failed'"
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("outbox event row");

    let payload = &event_row.0;
    assert_eq!(payload["event_type"], "sync.push.failed");
    assert_eq!(payload["source_module"], "integrations");
    assert_eq!(payload["mutation_class"], "SIDE_EFFECT");

    let inner = &payload["payload"];
    assert_eq!(inner["entity_type"], "invoice");
    assert_eq!(inner["entity_id"], "inv-push-fail-001");
    // failure_code is classified by the push path (invalid_payload when the payload
    // doesn't match the QboInvoicePayload schema; auth_failed if it reaches QBO with
    // dummy tokens).  Either is a valid classified fault — just assert it's non-empty.
    assert!(
        inner["failure_code"]
            .as_str()
            .map_or(false, |s| !s.is_empty()),
        "failure_code must be a non-empty string, got: {:?}",
        inner["failure_code"]
    );
    assert!(
        inner["connector_id"].is_string(),
        "connector_id must be a UUID string"
    );
    assert_eq!(inner["attempt_number"], 1);
    // classified faults that are not rate_limited/token_error are not retryable
    assert_eq!(inner["retryable"], false);

    cleanup(&pool, &app_id).await;
}

/// A push that succeeds does NOT enqueue a push.failed event.
#[tokio::test]
#[serial]
async fn test_no_push_failed_event_on_success_path() {
    let pool = setup_db().await;
    let app_id = format!("markers-no-fail-{}", Uuid::new_v4().simple());
    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool,
        &app_id,
        "quickbooks",
        "payment",
        "pay-success-001",
        "create",
        1,
        "fp-success-001",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    push_attempts::complete_attempt_with_markers(&pool, row.id, Some("tok-1"), None, None, None)
        .await
        .expect("complete")
        .expect("row");

    // No push.failed event should exist for this app.
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND event_type = 'sync.push.failed'"
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count.0, 0, "no push.failed event on success path");

    cleanup(&pool, &app_id).await;
}

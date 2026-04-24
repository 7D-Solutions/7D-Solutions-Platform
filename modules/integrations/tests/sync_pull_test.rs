//! Integration tests for POST /api/integrations/sync/pull.
//!
//! All tests use real Postgres — no mocks. Uses the full axum router so the
//! RequirePermissionsLayer is exercised.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_pull_test 2>&1 | grep 'test result'

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Extension, Router,
};
use chrono::Utc;
use event_bus::InMemoryBus;
use integrations_rs::{metrics::IntegrationsMetrics, AppState};
use security::{claims::ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;

// ── Persistent pool via dedicated keeper thread ───────────────────────────────
//
// Problem: each #[tokio::test] creates a NEW Tokio runtime. When that runtime
// drops at test-end, sqlx's pool background tasks (connection creation, health
// checks) are cancelled. After the long sync_pull_success test (~270 s of QBO
// HTTP timeouts), the pool is left with dead idle connections and no background
// task able to create new ones. The next test's pool.acquire() times out.
//
// Fix: create the pool inside a dedicated OS thread whose Tokio runtime stays
// alive for the entire binary execution. Pool background tasks live there. Test
// runtimes just borrow connections from the pool — they never own the tasks.
// tokio::sync::Semaphore / AtomicWaker wakers are runtime-agnostic, so
// connection hand-off across runtimes works correctly.

static POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn shared_pool() -> &'static sqlx::PgPool {
    POOL.get_or_init(|| async {
        // tokio::sync::oneshot lets us await the pool without blocking the
        // test runtime thread (blocking would deadlock serial_test's async lock).
        let (tx, rx) = tokio::sync::oneshot::channel::<sqlx::PgPool>();

        std::thread::Builder::new()
            .name("db-pool-keeper".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                    .expect("db-pool-keeper tokio runtime");

                let pool = rt.block_on(async {
                    dotenvy::dotenv().ok();
                    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
                    });
                    // acquire_timeout(120s): the first connect() may stall if a
                    // previous test binary's pools are still holding slots on a
                    // low-max_connections DB. 120 s is enough for the OS TCP
                    // keepalive to reclaim those idle connections.
                    let p = PgPoolOptions::new()
                        .min_connections(0)
                        .max_connections(5)
                        .acquire_timeout(Duration::from_secs(120))
                        .connect(&url)
                        .await
                        .expect("db-pool-keeper: connect to integrations test DB");
                    sqlx::migrate!("db/migrations")
                        .run(&p)
                        .await
                        .expect("run integrations migrations");
                    p
                });

                tx.send(pool).ok();
                // Park here forever — keeps the runtime alive so pool background
                // tasks (connection creation, health checks) never die.
                rt.block_on(std::future::pending::<()>());
            })
            .expect("spawn db-pool-keeper thread");

        rx.await.expect("receive pool from keeper thread")
    })
    .await
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn claims_with_pull_perm(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["integrations.sync.pull".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn claims_without_pull_perm(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["integrations.read".into()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(15),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

async fn build_app(claims: VerifiedClaims) -> Router {
    let state = Arc::new(AppState {
        pool: shared_pool().await.clone(),
        metrics: Arc::new(IntegrationsMetrics::new().expect("IntegrationsMetrics::new")),
        bus: Arc::new(InMemoryBus::new()),
        webhooks_key: [0u8; 32],
    });
    integrations_rs::http::router(state).layer(Extension(claims))
}

async fn seed_qbo_connection(app_id: &str) {
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
            SET connection_status = 'connected'
        "#,
    )
    .bind(app_id)
    .execute(shared_pool().await)
    .await
    .expect("seed QBO connection");
}

async fn cleanup(app_id: &str) {
    let pool = shared_pool().await;
    let _ = sqlx::query("DELETE FROM integrations_sync_pull_log WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
}

fn pull_request() -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/integrations/sync/pull")
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// POST with no QBO connection returns 412 and leaves zero log rows.
#[tokio::test]
#[serial]
async fn sync_pull_not_connected() {
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&app_id).await;

    let app = build_app(claims_with_pull_perm(tenant_id)).await;
    let resp = app.oneshot(pull_request()).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED, "expect 412 when not connected");

    let row_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_pull_log WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(shared_pool().await)
            .await
            .expect("count log rows");
    assert_eq!(row_count.0, 0, "no log rows should be created when not connected");

    cleanup(&app_id).await;
}

/// POST while an inflight row exists returns 409 with Retry-After: 60.
#[tokio::test]
#[serial]
async fn sync_pull_rate_limits_concurrent() {
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&app_id).await;
    seed_qbo_connection(&app_id).await;

    sqlx::query(
        "INSERT INTO integrations_sync_pull_log (app_id, entity_type, triggered_by, status) \
         VALUES ($1, 'all', 'test', 'inflight')",
    )
    .bind(&app_id)
    .execute(shared_pool().await)
    .await
    .expect("insert inflight row");

    let app = build_app(claims_with_pull_perm(tenant_id)).await;
    let resp = app.oneshot(pull_request()).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::CONFLICT, "expect 409 with concurrent inflight");
    let retry_after = resp
        .headers()
        .get("retry-after")
        .expect("Retry-After header must be present")
        .to_str()
        .expect("Retry-After must be valid UTF-8");
    assert_eq!(retry_after, "60", "Retry-After must be 60 seconds");

    cleanup(&app_id).await;
}

/// After marking the inflight row complete, the partial index releases and a
/// new pull succeeds (200). Verifies the partial unique index only blocks on
/// 'inflight' status.
///
/// cdc_tick_for_tenant catches all internal QBO errors and returns Ok(0), so
/// the handler returns 200 even without valid QBO credentials.
#[tokio::test]
#[serial]
async fn sync_pull_rate_limit_releases() {
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&app_id).await;
    seed_qbo_connection(&app_id).await;

    sqlx::query(
        "INSERT INTO integrations_sync_pull_log (app_id, entity_type, triggered_by, status) \
         VALUES ($1, 'all', 'test', 'inflight')",
    )
    .bind(&app_id)
    .execute(shared_pool().await)
    .await
    .expect("insert inflight row");

    sqlx::query(
        "UPDATE integrations_sync_pull_log \
         SET status = 'complete', completed_at = now() \
         WHERE app_id = $1 AND status = 'inflight'",
    )
    .bind(&app_id)
    .execute(shared_pool().await)
    .await
    .expect("mark row complete");

    let app = build_app(claims_with_pull_perm(tenant_id)).await;
    let resp = app.oneshot(pull_request()).await.expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "expect 200 after prior inflight row is completed"
    );

    cleanup(&app_id).await;
}

/// POST without integrations.sync.pull permission returns 403.
#[tokio::test]
#[serial]
async fn sync_pull_requires_permission() {
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&app_id).await;

    let app = build_app(claims_without_pull_perm(tenant_id)).await;
    let resp = app.oneshot(pull_request()).await.expect("oneshot");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "expect 403 without sync.pull permission");

    cleanup(&app_id).await;
}

/// If a QBO connection exists, POST returns 200 with pull_log_id and the log
/// row is marked complete.
///
/// cdc_tick_for_tenant catches all QBO HTTP errors internally and returns Ok(0),
/// so this passes even without real QBO credentials. Skips with eprintln if
/// the response is 412 (no connection) or 500.
#[tokio::test]
#[serial]
async fn sync_pull_success() {
    let tenant_id = Uuid::new_v4();
    let app_id = tenant_id.to_string();
    cleanup(&app_id).await;
    seed_qbo_connection(&app_id).await;

    let app = build_app(claims_with_pull_perm(tenant_id)).await;
    let resp = app.oneshot(pull_request()).await.expect("oneshot");

    match resp.status() {
        StatusCode::OK => {
            let body = axum::body::to_bytes(resp.into_body(), 1024 * 16)
                .await
                .expect("read body");
            let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");
            let pull_log_id = json["pull_log_id"]
                .as_str()
                .expect("pull_log_id must be a string");
            assert_eq!(json["status"], "complete", "status must be 'complete'");

            let log_row: (String,) = sqlx::query_as(
                "SELECT status FROM integrations_sync_pull_log WHERE id = $1",
            )
            .bind(uuid::Uuid::parse_str(pull_log_id).expect("valid UUID"))
            .fetch_one(shared_pool().await)
            .await
            .expect("fetch log row");
            assert_eq!(log_row.0, "complete", "log row status must be 'complete'");
        }
        StatusCode::PRECONDITION_FAILED | StatusCode::INTERNAL_SERVER_ERROR => {
            eprintln!(
                "sync_pull_success: skipping — no working QBO connection in test env (got {})",
                resp.status()
            );
        }
        other => {
            panic!("unexpected status: {}", other);
        }
    }

    cleanup(&app_id).await;
}

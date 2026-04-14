//! E2E test for tenant-scoped connection budgets.
//!
//! This proves the SDK rejects the 6th concurrent request for one tenant
//! with a fast 429 + Retry-After while a different tenant can still acquire
//! a connection from the same pool.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use platform_http_contracts::ApiError;
use platform_sdk::{Manifest, ModuleContext, TenantPoolError};
use sqlx::postgres::PgPoolOptions;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Notify;
use tower::ServiceExt;
use uuid::Uuid;

mod common;

#[derive(Clone)]
struct AppState {
    ctx: Arc<ModuleContext>,
    started: Arc<AtomicUsize>,
    ready: Arc<Notify>,
    release: Arc<Notify>,
    target_holds: usize,
}

fn test_manifest() -> Manifest {
    Manifest::from_str(
        r#"
[module]
name = "noisy-neighbor"

[database]
migrations = "./db/migrations"

[database.tenant_quota]
max_connections = 5
"#,
        None,
    )
    .expect("manifest should parse")
}

async fn test_pool() -> sqlx::PgPool {
    let url = common::get_ar_db_url();
    PgPoolOptions::new()
        .max_connections(8)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&url)
        .await
        .expect("connect noisy-neighbor test pool")
}

async fn hold_handler(
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut guard = state
        .ctx
        .pool_for_tenant(tenant_id)
        .await
        .map_err(map_pool_error)?;

    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&mut **guard)
        .await
        .map_err(|e| ApiError::internal(format!("tenant {tenant_id} probe failed: {e}")))?;

    let started = state.started.fetch_add(1, Ordering::SeqCst) + 1;
    if started >= state.target_holds {
        state.ready.notify_waiters();
    }

    state.release.notified().await;
    Ok(StatusCode::OK)
}

async fn ping_handler(
    State(state): State<AppState>,
    Path(tenant_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut guard = state
        .ctx
        .pool_for_tenant(tenant_id)
        .await
        .map_err(map_pool_error)?;

    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&mut **guard)
        .await
        .map_err(|e| ApiError::internal(format!("tenant {tenant_id} ping failed: {e}")))?;

    Ok(StatusCode::OK)
}

fn map_pool_error(err: TenantPoolError) -> ApiError {
    match err {
        TenantPoolError::QuotaExceeded {
            tenant_id,
            max_connections,
        } => ApiError::too_many_requests(format!(
            "tenant {tenant_id} exceeded connection budget of {max_connections}"
        ))
        .with_retry_after_secs(1),
        TenantPoolError::UnknownTenant(tenant_id) => {
            ApiError::not_found(format!("unknown tenant {tenant_id}"))
        }
        TenantPoolError::Pool(message) => ApiError::internal(message),
    }
}

#[tokio::test]
async fn tenant_budget_isolated_under_load() {
    let pool = test_pool().await;
    let ctx = Arc::new(ModuleContext::new(pool, test_manifest(), None));

    let state = AppState {
        ctx,
        started: Arc::new(AtomicUsize::new(0)),
        ready: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
        target_holds: 5,
    };

    let app = Router::new()
        .route("/tenant/{tenant_id}/hold", get(hold_handler))
        .route("/tenant/{tenant_id}/ping", get(ping_handler))
        .with_state(state.clone());

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let mut hold_tasks = Vec::new();
    for _ in 0..5 {
        let app = app.clone();
        let uri = format!("/tenant/{tenant_a}/hold");
        hold_tasks.push(tokio::spawn(async move {
            app.oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("build hold request"),
            )
            .await
            .expect("hold request")
        }));
    }

    loop {
        if state.started.load(Ordering::SeqCst) >= 5 {
            break;
        }
        state.ready.notified().await;
    }

    let b_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/tenant/{tenant_b}/ping"))
                .body(Body::empty())
                .expect("build tenant B request"),
        )
        .await
        .expect("tenant B request");
    assert_eq!(
        b_resp.status(),
        StatusCode::OK,
        "tenant B should still succeed"
    );

    let a_blocked = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/tenant/{tenant_a}/hold"))
                .body(Body::empty())
                .expect("build blocked request"),
        )
        .await
        .expect("blocked request");
    assert_eq!(
        a_blocked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "tenant A's 6th request must be rejected"
    );

    let retry_after = a_blocked
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok());
    assert_eq!(
        retry_after,
        Some("1"),
        "tenant A must receive Retry-After: 1"
    );

    state.release.notify_waiters();

    for task in hold_tasks {
        let resp = task.await.expect("join hold task");
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

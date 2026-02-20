/// Route definitions for the control-plane service
///
/// Exposes:
///   POST /api/control/tenants                          — Create and provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary       — Read-only tenant summary (via tenant-registry)
///   GET  /api/control/tenants/:tenant_id/retention     — Read retention policy
///   PUT  /api/control/tenants/:tenant_id/retention     — Upsert retention policy
///   POST /api/control/tenants/:tenant_id/tombstone     — Tombstone tenant data (audited)
///   POST /api/control/platform-billing-runs            — Run the platform billing cycle for a period

use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use std::sync::Arc;
use tenant_registry::routes::{summary_router, entitlements_router, status_router, SummaryState};

use crate::handlers;
use crate::state::AppState;

/// GET /api/ready — standardized readiness probe
async fn ready(
    State(state): State<Arc<AppState>>,
) -> Result<Json<health::ReadyResponse>, (StatusCode, Json<health::ReadyResponse>)> {
    let start = std::time::Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let resp = health::build_ready_response(
        "control-plane",
        env!("CARGO_PKG_VERSION"),
        vec![health::db_check(latency, db_err)],
    );
    health::ready_response_to_axum(resp)
}

/// Build the full control-plane router.
pub fn build_router(state: Arc<AppState>, summary_state: Arc<SummaryState>) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/ready", get(ready))
        .route(
            "/api/control/tenants",
            post(handlers::create_tenant),
        )
        .route(
            "/api/control/tenants/:tenant_id/retention",
            get(handlers::get_retention).put(handlers::set_retention),
        )
        .route(
            "/api/control/tenants/:tenant_id/tombstone",
            post(handlers::tombstone_tenant),
        )
        .route(
            "/api/control/platform-billing-runs",
            post(handlers::platform_billing_run),
        )
        .with_state(state)
        .merge(summary_router(summary_state.clone()))
        .merge(entitlements_router(summary_state.clone()))
        .merge(status_router(summary_state))
}

/// Build only the provisioning router (for testing without summary state)
pub fn provisioning_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/control/tenants",
            post(handlers::create_tenant),
        )
        .with_state(state)
}

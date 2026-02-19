/// Route definitions for the control-plane service
///
/// Exposes:
///   POST /api/control/tenants                          — Create and provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary       — Read-only tenant summary (via tenant-registry)
///   GET  /api/control/tenants/:tenant_id/retention     — Read retention policy
///   PUT  /api/control/tenants/:tenant_id/retention     — Upsert retention policy
///   POST /api/control/tenants/:tenant_id/tombstone     — Tombstone tenant data (audited)
///   POST /api/control/platform-billing-runs            — Run the platform billing cycle for a period

use axum::{routing::{get, post}, Router};
use std::sync::Arc;
use tenant_registry::routes::{summary_router, entitlements_router, status_router, SummaryState};

use crate::handlers;
use crate::state::AppState;

/// Build the full control-plane router.
pub fn build_router(state: Arc<AppState>, summary_state: Arc<SummaryState>) -> Router {
    Router::new()
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

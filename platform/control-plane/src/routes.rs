/// Route definitions for the control-plane service
///
/// Exposes:
///   POST /api/control/tenants   — Create and provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary  — Read-only tenant summary (via tenant-registry)

use axum::{routing::post, Router};
use std::sync::Arc;
use tenant_registry::routes::{summary_router, SummaryState};

use crate::handlers;
use crate::state::AppState;

/// Build the full control-plane router.
///
/// Merges:
/// - POST /api/control/tenants (provisioning)
/// - GET  /api/control/tenants/:tenant_id/summary (from tenant-registry summary_router)
pub fn build_router(state: Arc<AppState>, summary_state: Arc<SummaryState>) -> Router {
    Router::new()
        .route(
            "/api/control/tenants",
            post(handlers::create_tenant),
        )
        .with_state(state)
        .merge(summary_router(summary_state))
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

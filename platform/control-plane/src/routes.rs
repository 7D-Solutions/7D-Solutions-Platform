/// Route definitions for the control-plane service
///
/// Exposes:
///   POST /api/control/tenants                          — Create and provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary       — Read-only tenant summary (via tenant-registry)
///   GET  /api/control/tenants/:tenant_id/retention     — Read retention policy
///   PUT  /api/control/tenants/:tenant_id/retention     — Upsert retention policy
///   POST /api/control/tenants/:tenant_id/tombstone     — Tombstone tenant data (audited)
///   POST /api/control/tenants/:tenant_id/gdpr-erasure  — GDPR-friendly alias for tombstone
///   POST /api/control/platform-billing-runs            — Run the platform billing cycle for a period
///   GET  /api/tenants/:tenant_id/app-id               — Resolve tenant_id → app_id (for TTP billing)
///   GET  /api/ttp/plans                               — List platform billing plans (plan catalog)
///   GET  /api/tenants                                 — Paginated tenant list (BFF-compatible)
///   GET  /api/tenants/:tenant_id                      — Tenant detail with derived name and seat_limit
///   GET  /api/service-catalog                          — Module-to-URL service catalog
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use security::{optional_claims_mw, permissions, RequirePermissionsLayer};
use std::sync::Arc;
use tenant_registry::plans_router;
use tenant_registry::routes::{
    app_id_router, entitlements_router, status_router, summary_router, SummaryState,
};
use tenant_registry::{tenant_detail_router, tenant_list_router};

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

    let pool_metrics = health::PoolMetrics {
        size: state.pool.size(),
        idle: state.pool.num_idle() as u32,
        active: state
            .pool
            .size()
            .saturating_sub(state.pool.num_idle() as u32),
    };

    let resp = health::build_ready_response(
        "control-plane",
        env!("CARGO_PKG_VERSION"),
        vec![health::db_check_with_pool(latency, db_err, pool_metrics)],
    );
    health::ready_response_to_axum(resp)
}

/// Build the full control-plane router.
pub fn build_router(state: Arc<AppState>, summary_state: Arc<SummaryState>) -> Router {
    let verifier = state.jwt_verifier.clone();

    Router::new()
        // RBAC-protected: create-tenant requires PLATFORM_TENANTS_CREATE
        .route("/api/control/tenants", post(handlers::create_tenant))
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::PLATFORM_TENANTS_CREATE,
        ]))
        // Unprotected routes (no per-route permission gate)
        .route("/healthz", get(health::healthz))
        .route("/api/ready", get(ready))
        .route("/api/health", get(ready))
        .route(
            "/api/control/tenants/{tenant_id}/retention",
            get(handlers::get_retention).put(handlers::set_retention),
        )
        .route(
            "/api/control/tenants/{tenant_id}/tombstone",
            post(handlers::tombstone_tenant),
        )
        .route(
            "/api/control/tenants/{tenant_id}/gdpr-erasure",
            post(handlers::gdpr_erasure),
        )
        .route(
            "/api/control/platform-billing-runs",
            post(handlers::platform_billing_run),
        )
        .route(
            "/api/control/tenants/{tenant_id}/provisioning",
            get(handlers::provisioning_status),
        )
        .route(
            "/api/control/tenants/{tenant_id}/retry",
            post(handlers::retry_provisioning),
        )
        .route("/api/service-catalog", get(handlers::service_catalog))
        .with_state(state)
        .merge(summary_router(summary_state.clone()))
        .merge(entitlements_router(summary_state.clone()))
        .merge(status_router(summary_state.clone()))
        .merge(app_id_router(summary_state.clone()))
        .merge(plans_router(summary_state.pool.clone()))
        .merge(tenant_list_router(summary_state.pool.clone()))
        .merge(tenant_detail_router(summary_state.pool.clone()))
        // Outermost layer: extract JWT claims for all routes.
        // optional_claims_mw inserts VerifiedClaims into extensions when a valid
        // Bearer token is present; requests without a token pass through unchallenged,
        // but RequirePermissionsLayer will then return 401.
        .layer(axum::middleware::from_fn_with_state(
            verifier,
            optional_claims_mw,
        ))
}

/// Build only the provisioning router (for testing without summary state)
pub fn provisioning_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/control/tenants", post(handlers::create_tenant))
        .route(
            "/api/control/tenants/{tenant_id}/provisioning",
            get(handlers::provisioning_status),
        )
        .with_state(state)
}

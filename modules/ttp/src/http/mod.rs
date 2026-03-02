pub mod billing;
pub mod metering;
pub mod service_agreements;

use axum::{
    routing::{get, post},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the TTP HTTP router with all endpoints.
///
/// Mutation routes (POST) require the `ttp.mutate` permission in the
/// caller's JWT.  Read routes are unenforced at this stage.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        // Billing runs — write
        .route("/api/ttp/billing-runs", post(billing::create_billing_run))
        // Metering — write
        .route("/api/metering/events", post(metering::ingest_events))
        .route_layer(RequirePermissionsLayer::new(&[permissions::TTP_MUTATE]))
        .with_state(state.clone());

    let reads = Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Metering — read
        .route("/api/metering/trace", get(metering::get_trace))
        // Service agreements — read
        .route(
            "/api/ttp/service-agreements",
            get(service_agreements::list_service_agreements),
        )
        .with_state(state);

    Router::new().merge(mutations).merge(reads)
}

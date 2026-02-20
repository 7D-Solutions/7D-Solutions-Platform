pub mod billing;
pub mod metering;

use axum::{routing::{get, post}, Router};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the TTP HTTP router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Billing runs
        .route("/api/ttp/billing-runs", post(billing::create_billing_run))
        // Metering
        .route("/api/metering/events", post(metering::ingest_events))
        .route("/api/metering/trace", get(metering::get_trace))
        .with_state(state)
}

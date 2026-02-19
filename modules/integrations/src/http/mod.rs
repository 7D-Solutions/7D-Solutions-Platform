use axum::{
    routing::get,
    Router,
};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Integrations HTTP router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ops
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(state)
}

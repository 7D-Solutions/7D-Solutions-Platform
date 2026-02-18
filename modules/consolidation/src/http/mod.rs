use axum::{routing::get, Router};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Consolidation HTTP router.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
}

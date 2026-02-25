use axum::{routing::get, Router};
use std::sync::Arc;

use crate::http;
use crate::AppState;

/// Read routes — accessible with any valid JWT (no extra permissions).
pub fn build_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
}

/// Mutation routes — caller must apply RequirePermissionsLayer externally.
/// Currently empty; future beads will add POST/PUT/PATCH/DELETE endpoints here.
pub fn build_mutation_router() -> Router<Arc<AppState>> {
    Router::new()
}

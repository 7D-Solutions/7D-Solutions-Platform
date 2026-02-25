use axum::{routing::get, Router};
use std::sync::Arc;

use crate::http;
use crate::AppState;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .with_state(state)
}

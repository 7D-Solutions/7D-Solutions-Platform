use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::handlers::AppState;

pub fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/documents", post(crate::handlers::create_document))
        .route("/api/documents", get(crate::handlers::list_documents))
        .route("/api/documents/{id}", get(crate::handlers::get_document))
        .route(
            "/api/documents/{id}/release",
            post(crate::handlers::release_document),
        )
        .route(
            "/api/documents/{id}/supersede",
            post(crate::handlers::supersede_document),
        )
        .route(
            "/api/documents/{id}/revisions",
            post(crate::handlers::create_revision),
        )
        .with_state(state)
}

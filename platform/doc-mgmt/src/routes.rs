use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::handlers::AppState;

pub fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Document lifecycle
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
        .route(
            "/api/documents/{id}/distributions",
            post(crate::distribution::create_distribution),
        )
        .route(
            "/api/documents/{id}/distributions",
            get(crate::distribution::list_distributions),
        )
        .route(
            "/api/distributions/{id}/status",
            post(crate::distribution::update_distribution_status),
        )
        // Retention policies
        .route(
            "/api/retention-policies",
            post(crate::retention::set_retention_policy),
        )
        .route(
            "/api/retention-policies/{doc_type}",
            get(crate::retention::get_retention_policy),
        )
        // Legal holds
        .route(
            "/api/documents/{id}/holds",
            get(crate::retention::list_holds),
        )
        .route(
            "/api/documents/{id}/holds/apply",
            post(crate::retention::apply_hold),
        )
        .route(
            "/api/documents/{id}/holds/release",
            post(crate::retention::release_hold),
        )
        // Disposal
        .route(
            "/api/documents/{id}/dispose",
            post(crate::retention::dispose_document),
        )
        // Templates
        .route(
            "/api/templates",
            post(crate::template_engine::create_template),
        )
        .route(
            "/api/templates/{id}",
            get(crate::template_engine::get_template),
        )
        .route(
            "/api/templates/{id}/render",
            post(crate::template_engine::render_template),
        )
        // Render artifacts
        .route(
            "/api/artifacts/{id}",
            get(crate::template_engine::get_artifact),
        )
        .with_state(state)
}

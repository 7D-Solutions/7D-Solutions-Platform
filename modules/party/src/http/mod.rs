use axum::{
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

pub mod party;

/// Build the Party HTTP router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ops
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Party CRUD
        .route("/api/party/companies", post(party::create_company))
        .route("/api/party/individuals", post(party::create_individual))
        .route("/api/party/parties", get(party::list_parties))
        .route("/api/party/parties/search", get(party::search_parties))
        .route("/api/party/parties/:id", get(party::get_party))
        .route("/api/party/parties/:id", put(party::update_party))
        .route("/api/party/parties/:id/deactivate", post(party::deactivate_party))
        .with_state(state)
}

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

pub mod addresses;
pub mod contacts;
pub mod party;

/// Build the Party HTTP router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Party CRUD
        .route("/api/party/companies", post(party::create_company))
        .route("/api/party/individuals", post(party::create_individual))
        .route("/api/party/parties", get(party::list_parties))
        .route("/api/party/parties/search", get(party::search_parties))
        .route("/api/party/parties/{id}", get(party::get_party))
        .route("/api/party/parties/{id}", put(party::update_party))
        .route("/api/party/parties/{id}/deactivate", post(party::deactivate_party))
        // Contact CRUD
        .route("/api/party/parties/{party_id}/contacts", post(contacts::create_contact))
        .route("/api/party/parties/{party_id}/contacts", get(contacts::list_contacts))
        .route("/api/party/contacts/{id}", get(contacts::get_contact))
        .route("/api/party/contacts/{id}", put(contacts::update_contact))
        .route("/api/party/contacts/{id}", delete(contacts::delete_contact))
        // Address CRUD
        .route("/api/party/parties/{party_id}/addresses", post(addresses::create_address))
        .route("/api/party/parties/{party_id}/addresses", get(addresses::list_addresses))
        .route("/api/party/addresses/{id}", get(addresses::get_address))
        .route("/api/party/addresses/{id}", put(addresses::update_address))
        .route("/api/party/addresses/{id}", delete(addresses::delete_address))
        .with_state(state)
}

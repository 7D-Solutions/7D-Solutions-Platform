use axum::{
    routing::{delete, get, post, put},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

pub mod addresses;
pub mod contacts;
pub mod party;

/// Build the Party HTTP router with all endpoints.
///
/// Mutation routes (POST / PUT / DELETE) require the `party.mutate`
/// permission in the caller's JWT.  Read routes are unenforced at this stage.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        // Party — write
        .route("/api/party/companies", post(party::create_company))
        .route("/api/party/individuals", post(party::create_individual))
        .route("/api/party/parties/{id}", put(party::update_party))
        .route(
            "/api/party/parties/{id}/deactivate",
            post(party::deactivate_party),
        )
        // Contact — write
        .route(
            "/api/party/parties/{party_id}/contacts",
            post(contacts::create_contact),
        )
        .route("/api/party/contacts/{id}", put(contacts::update_contact))
        .route("/api/party/contacts/{id}", delete(contacts::delete_contact))
        // Address — write
        .route(
            "/api/party/parties/{party_id}/addresses",
            post(addresses::create_address),
        )
        .route("/api/party/addresses/{id}", put(addresses::update_address))
        .route(
            "/api/party/addresses/{id}",
            delete(addresses::delete_address),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::PARTY_MUTATE]))
        .with_state(state.clone());

    let reads = Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Party — read
        .route("/api/party/parties", get(party::list_parties))
        .route("/api/party/parties/search", get(party::search_parties))
        .route("/api/party/parties/{id}", get(party::get_party))
        // Contact — read
        .route(
            "/api/party/parties/{party_id}/contacts",
            get(contacts::list_contacts),
        )
        .route("/api/party/contacts/{id}", get(contacts::get_contact))
        // Address — read
        .route(
            "/api/party/parties/{party_id}/addresses",
            get(addresses::list_addresses),
        )
        .route("/api/party/addresses/{id}", get(addresses::get_address))
        .with_state(state);

    Router::new().merge(mutations).merge(reads)
}

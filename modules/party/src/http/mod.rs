use axum::{
    routing::{delete, get, post, put},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::AppState;

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
        .route(
            "/api/party/parties/{party_id}/contacts/{id}/set-primary",
            post(contacts::set_primary),
        )
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
        .route(
            "/api/party/parties/{party_id}/primary-contacts",
            get(contacts::primary_contacts),
        )
        // Address — read
        .route(
            "/api/party/parties/{party_id}/addresses",
            get(addresses::list_addresses),
        )
        .route("/api/party/addresses/{id}", get(addresses::get_address))
        .route_layer(RequirePermissionsLayer::new(&[permissions::PARTY_READ]))
        .with_state(state.clone());

    Router::new().merge(mutations).merge(reads)
}

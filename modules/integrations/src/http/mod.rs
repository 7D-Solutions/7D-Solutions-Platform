pub mod connectors;
pub mod external_refs;
pub mod webhooks;

use axum::{
    routing::{get, post},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Integrations HTTP router with all endpoints.
///
/// Mutation routes (POST / PUT / DELETE) require the `integrations.mutate`
/// permission in the caller's JWT.  Read routes are unenforced at this stage.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        // Inbound webhooks — write
        .route(
            "/api/webhooks/inbound/{system}",
            post(webhooks::inbound_webhook),
        )
        // External refs — write
        .route(
            "/api/integrations/external-refs",
            post(external_refs::create_external_ref),
        )
        .route(
            "/api/integrations/external-refs/{id}",
            axum::routing::put(external_refs::update_external_ref)
                .delete(external_refs::delete_external_ref),
        )
        // Connectors — write
        .route(
            "/api/integrations/connectors",
            post(connectors::register_connector),
        )
        .route(
            "/api/integrations/connectors/{id}/test",
            post(connectors::run_connector_test),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::INTEGRATIONS_MUTATE]))
        .with_state(state.clone());

    let reads = Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // External refs — read
        .route(
            "/api/integrations/external-refs/by-entity",
            get(external_refs::list_by_entity),
        )
        .route(
            "/api/integrations/external-refs/by-system",
            get(external_refs::get_by_external),
        )
        .route(
            "/api/integrations/external-refs/{id}",
            get(external_refs::get_external_ref),
        )
        // Connectors — read
        .route(
            "/api/integrations/connectors/types",
            get(connectors::list_connector_types),
        )
        .route(
            "/api/integrations/connectors",
            get(connectors::list_connectors),
        )
        .route(
            "/api/integrations/connectors/{id}",
            get(connectors::get_connector),
        )
        .with_state(state);

    Router::new().merge(mutations).merge(reads)
}

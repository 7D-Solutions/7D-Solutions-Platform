pub mod connectors;
pub mod external_refs;
pub mod internal;
pub mod oauth;
pub mod qbo_invoice;
pub mod sync;
pub mod webhooks;

use axum::{
    routing::{get, post},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::AppState;

/// Build the Integrations HTTP router with all endpoints.
///
/// Mutation routes (POST / PUT / DELETE) require the `integrations.mutate`
/// permission in the caller's JWT.  Business read routes require the
/// `integrations.read` permission.  Ops endpoints (health, ready, version,
/// metrics) and the inbound webhook endpoint are unauthenticated.
pub fn router(state: Arc<AppState>) -> Router {
    let webhook_inbound = Router::new()
        // Inbound webhooks — unauthenticated (gated by signature verification in handler)
        .route(
            "/api/webhooks/inbound/{system}",
            post(webhooks::inbound_webhook),
        )
        .with_state(state.clone());

    // OAuth callback — unauthenticated (redirect from provider)
    let oauth_callback = Router::new()
        .route(
            "/api/integrations/oauth/callback/{provider}",
            get(oauth::callback),
        )
        .with_state(state.clone());

    let mutations = Router::new()
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
        // OAuth — write
        .route(
            "/api/integrations/oauth/connect/{provider}",
            get(oauth::connect),
        )
        .route(
            "/api/integrations/oauth/disconnect/{provider}",
            post(oauth::disconnect),
        )
        // QBO invoice — write
        .route(
            "/api/integrations/qbo/invoice/{invoice_id}/update",
            post(qbo_invoice::update_invoice),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_MUTATE,
        ]))
        .with_state(state.clone());

    let reads = Router::new()
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
        // OAuth — read
        .route(
            "/api/integrations/oauth/status/{provider}",
            get(oauth::status),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_READ,
        ]))
        .with_state(state.clone());

    // Sync — authority flip (financially sensitive; requires dedicated permission)
    let sync_authority = Router::new()
        .route(
            "/api/integrations/sync/authority",
            post(sync::flip_authority),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_SYNC_AUTHORITY_FLIP,
        ]))
        .with_state(state.clone());

    // Sync — conflict resolution (single-item and bulk)
    let sync_conflict_resolve = Router::new()
        .route(
            "/api/integrations/sync/conflicts/{id}/resolve",
            post(sync::resolve_conflict),
        )
        .route(
            "/api/integrations/sync/conflicts/bulk-resolve",
            post(sync::bulk_resolve_conflicts_handler),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_SYNC_CONFLICT_RESOLVE,
        ]))
        .with_state(state.clone());

    // Sync — push
    let sync_push = Router::new()
        .route(
            "/api/integrations/sync/push/{entity_type}",
            post(sync::push_entity),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_SYNC_PUSH,
        ]))
        .with_state(state.clone());

    // Sync — reads (authority state, conflicts, dlq, push-attempts, jobs)
    let sync_reads = Router::new()
        .route(
            "/api/integrations/sync/authority",
            get(sync::get_authority_state),
        )
        .route(
            "/api/integrations/sync/conflicts",
            get(sync::list_conflicts),
        )
        .route("/api/integrations/sync/dlq", get(sync::list_dlq))
        .route(
            "/api/integrations/sync/push-attempts",
            get(sync::list_push_attempts),
        )
        .route("/api/integrations/sync/jobs", get(sync::list_jobs))
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_SYNC_READ,
        ]))
        .with_state(state.clone());

    // OAuth admin — token import (dev/CI seeding; admin permission + env gate)
    let oauth_admin = Router::new()
        .route(
            "/api/integrations/oauth/import",
            post(oauth::import_tokens),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_OAUTH_ADMIN,
        ]))
        .with_state(state.clone());

    // Sync admin — CDC trigger (dev-local only; handler enforces APP_PROFILE guard)
    let sync_admin = Router::new()
        .route(
            "/api/integrations/sync/cdc/trigger",
            post(sync::trigger_cdc),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INTEGRATIONS_OAUTH_ADMIN,
        ]))
        .with_state(state.clone());

    // Internal platform-to-platform endpoints — no auth layer, network-gated only
    let internal_routes = Router::new()
        .route(
            "/api/integrations/internal/carrier-credentials/{connector_type}",
            get(internal::get_carrier_credentials),
        )
        .with_state(state);

    Router::new()
        .merge(mutations)
        .merge(reads)
        .merge(sync_authority)
        .merge(sync_conflict_resolve)
        .merge(sync_push)
        .merge(sync_reads)
        .merge(webhook_inbound)
        .merge(oauth_callback)
        .merge(oauth_admin)
        .merge(sync_admin)
        .merge(internal_routes)
}

pub mod admin;
pub mod config;
pub mod consolidate;
pub mod intercompany;
pub mod statements;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Consolidation HTTP router.
///
/// Mutation routes (POST / PUT / DELETE) require the `consolidation.mutate`
/// permission in the caller's JWT.  Read routes are unenforced at this stage.
pub fn router() -> Router<Arc<AppState>> {
    let mutations: Router<Arc<AppState>> = Router::new()
        // Consolidation engine — write
        .route(
            "/api/consolidation/groups/{group_id}/consolidate",
            post(consolidate::run_consolidation),
        )
        // Groups — write
        .route("/api/consolidation/groups", post(config::create_group))
        .route("/api/consolidation/groups/{id}", put(config::update_group))
        .route("/api/consolidation/groups/{id}", delete(config::delete_group))
        // Entities — write
        .route(
            "/api/consolidation/groups/{group_id}/entities",
            post(config::create_entity),
        )
        .route("/api/consolidation/entities/{id}", put(config::update_entity))
        .route("/api/consolidation/entities/{id}", delete(config::delete_entity))
        // COA mappings — write
        .route(
            "/api/consolidation/groups/{group_id}/coa-mappings",
            post(config::create_coa_mapping),
        )
        .route(
            "/api/consolidation/coa-mappings/{id}",
            delete(config::delete_coa_mapping),
        )
        // Elimination rules — write
        .route(
            "/api/consolidation/groups/{group_id}/elimination-rules",
            post(config::create_elimination_rule),
        )
        .route(
            "/api/consolidation/elimination-rules/{id}",
            put(config::update_elimination_rule),
        )
        .route(
            "/api/consolidation/elimination-rules/{id}",
            delete(config::delete_elimination_rule),
        )
        // FX policies — write
        .route(
            "/api/consolidation/groups/{group_id}/fx-policies",
            put(config::upsert_fx_policy),
        )
        .route(
            "/api/consolidation/fx-policies/{id}",
            delete(config::delete_fx_policy),
        )
        // Intercompany — write
        .route(
            "/api/consolidation/groups/{group_id}/intercompany-match",
            post(intercompany::run_intercompany_match),
        )
        .route(
            "/api/consolidation/groups/{group_id}/eliminations",
            post(intercompany::post_eliminations),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::CONSOLIDATION_MUTATE]));

    let reads: Router<Arc<AppState>> = Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Consolidation engine — read
        .route(
            "/api/consolidation/groups/{group_id}/trial-balance",
            get(consolidate::get_consolidated_tb),
        )
        // Groups — read
        .route("/api/consolidation/groups", get(config::list_groups))
        .route("/api/consolidation/groups/{id}", get(config::get_group))
        .route(
            "/api/consolidation/groups/{id}/validate",
            get(config::validate_group),
        )
        // Entities — read
        .route(
            "/api/consolidation/groups/{group_id}/entities",
            get(config::list_entities),
        )
        .route("/api/consolidation/entities/{id}", get(config::get_entity))
        // COA mappings — read
        .route(
            "/api/consolidation/groups/{group_id}/coa-mappings",
            get(config::list_coa_mappings),
        )
        // Elimination rules — read
        .route(
            "/api/consolidation/groups/{group_id}/elimination-rules",
            get(config::list_elimination_rules),
        )
        .route(
            "/api/consolidation/elimination-rules/{id}",
            get(config::get_elimination_rule),
        )
        // FX policies — read
        .route(
            "/api/consolidation/groups/{group_id}/fx-policies",
            get(config::list_fx_policies),
        )
        // Financial statements — read
        .route(
            "/api/consolidation/groups/{group_id}/pl",
            get(statements::get_consolidated_pl),
        )
        .route(
            "/api/consolidation/groups/{group_id}/balance-sheet",
            get(statements::get_consolidated_bs),
        );

    Router::new().merge(mutations).merge(reads)
}

pub mod config;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Consolidation HTTP router.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Ops
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Groups
        .route("/api/consolidation/groups", post(config::create_group))
        .route("/api/consolidation/groups", get(config::list_groups))
        .route("/api/consolidation/groups/{id}", get(config::get_group))
        .route("/api/consolidation/groups/{id}", put(config::update_group))
        .route(
            "/api/consolidation/groups/{id}",
            delete(config::delete_group),
        )
        // Entities
        .route(
            "/api/consolidation/groups/{group_id}/entities",
            post(config::create_entity),
        )
        .route(
            "/api/consolidation/groups/{group_id}/entities",
            get(config::list_entities),
        )
        .route(
            "/api/consolidation/entities/{id}",
            get(config::get_entity),
        )
        .route(
            "/api/consolidation/entities/{id}",
            put(config::update_entity),
        )
        .route(
            "/api/consolidation/entities/{id}",
            delete(config::delete_entity),
        )
        // COA mappings
        .route(
            "/api/consolidation/groups/{group_id}/coa-mappings",
            post(config::create_coa_mapping),
        )
        .route(
            "/api/consolidation/groups/{group_id}/coa-mappings",
            get(config::list_coa_mappings),
        )
        .route(
            "/api/consolidation/coa-mappings/{id}",
            delete(config::delete_coa_mapping),
        )
        // Elimination rules
        .route(
            "/api/consolidation/groups/{group_id}/elimination-rules",
            post(config::create_elimination_rule),
        )
        .route(
            "/api/consolidation/groups/{group_id}/elimination-rules",
            get(config::list_elimination_rules),
        )
        .route(
            "/api/consolidation/elimination-rules/{id}",
            get(config::get_elimination_rule),
        )
        .route(
            "/api/consolidation/elimination-rules/{id}",
            put(config::update_elimination_rule),
        )
        .route(
            "/api/consolidation/elimination-rules/{id}",
            delete(config::delete_elimination_rule),
        )
        // FX policies
        .route(
            "/api/consolidation/groups/{group_id}/fx-policies",
            put(config::upsert_fx_policy),
        )
        .route(
            "/api/consolidation/groups/{group_id}/fx-policies",
            get(config::list_fx_policies),
        )
        .route(
            "/api/consolidation/fx-policies/{id}",
            delete(config::delete_fx_policy),
        )
        // Validation
        .route(
            "/api/consolidation/groups/{group_id}/validate",
            get(config::validate_group),
        )
}

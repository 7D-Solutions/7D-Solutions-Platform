use axum::{
    routing::{delete, get, post, put},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::AppState;

pub mod component_issue;
pub mod downtime;
pub mod fg_receipt;
pub mod operations;
pub mod pagination;
pub mod routings;
pub mod tenant;
pub mod time_entries;
pub mod work_orders;
pub mod workcenters;

/// Build the Production HTTP router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        .route(
            "/api/production/workcenters",
            post(workcenters::create_workcenter),
        )
        .route(
            "/api/production/workcenters/{id}",
            put(workcenters::update_workcenter),
        )
        .route(
            "/api/production/workcenters/{id}/deactivate",
            post(workcenters::deactivate_workcenter),
        )
        .route(
            "/api/production/work-orders",
            post(work_orders::create_work_order),
        )
        .route(
            "/api/production/work-orders/create",
            post(work_orders::composite_create_work_order),
        )
        .route(
            "/api/production/work-orders/{id}/release",
            post(work_orders::release_work_order),
        )
        .route(
            "/api/production/work-orders/{id}/close",
            post(work_orders::close_work_order),
        )
        .route(
            "/api/production/work-orders/{id}/component-issues",
            post(component_issue::post_component_issue),
        )
        .route(
            "/api/production/work-orders/{id}/fg-receipt",
            post(fg_receipt::post_fg_receipt),
        )
        .route(
            "/api/production/work-orders/{id}/operations/initialize",
            post(operations::initialize_operations),
        )
        .route(
            "/api/production/work-orders/{wo_id}/operations/{op_id}/start",
            post(operations::start_operation),
        )
        .route(
            "/api/production/work-orders/{wo_id}/operations/{op_id}/complete",
            post(operations::complete_operation),
        )
        .route(
            "/api/production/time-entries/start",
            post(time_entries::start_timer),
        )
        .route(
            "/api/production/time-entries/manual",
            post(time_entries::manual_entry),
        )
        .route(
            "/api/production/time-entries/{id}/stop",
            post(time_entries::stop_timer),
        )
        .route("/api/production/routings", post(routings::create_routing))
        .route(
            "/api/production/routings/{id}",
            put(routings::update_routing),
        )
        .route(
            "/api/production/routings/{id}/release",
            post(routings::release_routing),
        )
        .route(
            "/api/production/routings/{id}/steps",
            post(routings::add_routing_step),
        )
        .route(
            "/api/production/routings/{id}/steps/{step_id}",
            delete(routings::delete_routing_step),
        )
        .route(
            "/api/production/workcenters/{id}/downtime/start",
            post(downtime::start_downtime),
        )
        .route(
            "/api/production/downtime/{id}/end",
            post(downtime::end_downtime),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::PRODUCTION_MUTATE,
        ]))
        .with_state(state.clone());

    let reads = Router::new()
        .route(
            "/api/production/workcenters",
            get(workcenters::list_workcenters),
        )
        .route(
            "/api/production/workcenters/{id}",
            get(workcenters::get_workcenter),
        )
        .route(
            "/api/production/work-orders",
            get(work_orders::list_work_orders),
        )
        .route(
            "/api/production/work-orders/{id}",
            get(work_orders::get_work_order),
        )
        .route(
            "/api/production/work-orders/{id}/time-entries",
            get(time_entries::list_time_entries),
        )
        .route(
            "/api/production/work-orders/{id}/operations",
            get(operations::list_operations),
        )
        .route("/api/production/routings", get(routings::list_routings))
        .route(
            "/api/production/routings/by-item",
            get(routings::find_routings_by_item),
        )
        .route("/api/production/routings/{id}", get(routings::get_routing))
        .route(
            "/api/production/routings/{id}/steps",
            get(routings::list_routing_steps),
        )
        .route(
            "/api/production/routings/{id}/steps/{step_id}",
            get(routings::get_routing_step),
        )
        .route(
            "/api/production/workcenters/{id}/downtime",
            get(downtime::list_workcenter_downtime),
        )
        .route(
            "/api/production/downtime/active",
            get(downtime::list_active_downtime),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::PRODUCTION_READ,
        ]))
        .with_state(state.clone());

    Router::new().merge(mutations).merge(reads)
}

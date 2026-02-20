pub mod admin;
pub mod allocations;
pub mod approvals;
pub mod billing;
pub mod employees;
pub mod entries;
pub mod export;
pub mod projects;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Timekeeping HTTP router with all endpoints.
///
/// Mutation routes (POST / PUT / DELETE) require the `timekeeping.mutate`
/// permission in the caller's JWT.  Read routes are unenforced at this stage.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        // Employees — write
        .route("/api/timekeeping/employees", post(employees::create_employee))
        .route(
            "/api/timekeeping/employees/{id}",
            put(employees::update_employee).delete(employees::deactivate_employee),
        )
        // Projects — write
        .route("/api/timekeeping/projects", post(projects::create_project))
        .route(
            "/api/timekeeping/projects/{id}",
            put(projects::update_project).delete(projects::deactivate_project),
        )
        // Tasks — write
        .route("/api/timekeeping/tasks", post(projects::create_task))
        .route(
            "/api/timekeeping/tasks/{id}",
            put(projects::update_task).delete(projects::deactivate_task),
        )
        // Timesheet Entries — write
        .route("/api/timekeeping/entries", post(entries::create_entry))
        .route("/api/timekeeping/entries/correct", post(entries::correct_entry))
        .route("/api/timekeeping/entries/void", post(entries::void_entry))
        // Approvals — write
        .route("/api/timekeeping/approvals/submit", post(approvals::submit_approval))
        .route("/api/timekeeping/approvals/approve", post(approvals::approve_approval))
        .route("/api/timekeeping/approvals/reject", post(approvals::reject_approval))
        .route("/api/timekeeping/approvals/recall", post(approvals::recall_approval))
        // Allocations — write
        .route("/api/timekeeping/allocations", post(allocations::create_allocation))
        .route(
            "/api/timekeeping/allocations/{id}",
            put(allocations::update_allocation).delete(allocations::deactivate_allocation),
        )
        // Exports — write
        .route("/api/timekeeping/exports", post(export::create_export))
        // Billing rates + billing runs — write
        .route("/api/timekeeping/rates", post(billing::create_rate))
        .route("/api/timekeeping/billing-runs", post(billing::create_billing_run))
        .route_layer(RequirePermissionsLayer::new(&[permissions::TIMEKEEPING_MUTATE]))
        .with_state(state.clone());

    let reads = Router::new()
        // Ops
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Employees — read
        .route("/api/timekeeping/employees", get(employees::list_employees))
        .route("/api/timekeeping/employees/{id}", get(employees::get_employee))
        // Projects — read
        .route("/api/timekeeping/projects", get(projects::list_projects))
        .route("/api/timekeeping/projects/{id}", get(projects::get_project))
        .route("/api/timekeeping/projects/{project_id}/tasks", get(projects::list_tasks))
        .route("/api/timekeeping/tasks/{id}", get(projects::get_task))
        // Entries — read
        .route("/api/timekeeping/entries", get(entries::list_entries))
        .route("/api/timekeeping/entries/{entry_id}/history", get(entries::entry_history))
        // Approvals — read
        .route("/api/timekeeping/approvals", get(approvals::list_approvals))
        .route("/api/timekeeping/approvals/pending", get(approvals::list_pending))
        .route("/api/timekeeping/approvals/{id}", get(approvals::get_approval))
        .route("/api/timekeeping/approvals/{id}/actions", get(approvals::approval_actions))
        // Allocations — read
        .route("/api/timekeeping/allocations", get(allocations::list_allocations))
        .route("/api/timekeeping/allocations/{id}", get(allocations::get_allocation))
        .route("/api/timekeeping/rollups/by-project", get(allocations::rollup_by_project))
        .route("/api/timekeeping/rollups/by-employee", get(allocations::rollup_by_employee))
        .route(
            "/api/timekeeping/rollups/by-task/{project_id}",
            get(allocations::rollup_by_task),
        )
        // Exports — read
        .route("/api/timekeeping/exports", get(export::list_exports))
        .route("/api/timekeeping/exports/{id}", get(export::get_export))
        // Billing rates — read
        .route("/api/timekeeping/rates", get(billing::list_rates))
        .with_state(state);

    Router::new().merge(mutations).merge(reads)
}

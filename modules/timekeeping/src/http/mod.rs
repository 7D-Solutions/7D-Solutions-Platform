pub mod allocations;
pub mod approvals;
pub mod employees;
pub mod entries;
pub mod export;
pub mod projects;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::{metrics, ops, AppState};

/// Build the Timekeeping HTTP router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ops
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Employees
        .route(
            "/api/timekeeping/employees",
            post(employees::create_employee).get(employees::list_employees),
        )
        .route(
            "/api/timekeeping/employees/{id}",
            get(employees::get_employee)
                .put(employees::update_employee)
                .delete(employees::deactivate_employee),
        )
        // Projects
        .route(
            "/api/timekeeping/projects",
            post(projects::create_project).get(projects::list_projects),
        )
        .route(
            "/api/timekeeping/projects/{id}",
            get(projects::get_project)
                .put(projects::update_project)
                .delete(projects::deactivate_project),
        )
        // Timesheet Entries
        .route(
            "/api/timekeeping/entries",
            post(entries::create_entry).get(entries::list_entries),
        )
        .route(
            "/api/timekeeping/entries/correct",
            post(entries::correct_entry),
        )
        .route(
            "/api/timekeeping/entries/void",
            post(entries::void_entry),
        )
        .route(
            "/api/timekeeping/entries/{entry_id}/history",
            get(entries::entry_history),
        )
        // Tasks (nested under projects for listing, flat for direct access)
        .route(
            "/api/timekeeping/projects/{project_id}/tasks",
            get(projects::list_tasks),
        )
        .route(
            "/api/timekeeping/tasks",
            post(projects::create_task),
        )
        .route(
            "/api/timekeeping/tasks/{id}",
            get(projects::get_task)
                .put(projects::update_task)
                .delete(projects::deactivate_task),
        )
        // Approvals
        .route(
            "/api/timekeeping/approvals",
            get(approvals::list_approvals),
        )
        .route(
            "/api/timekeeping/approvals/pending",
            get(approvals::list_pending),
        )
        .route(
            "/api/timekeeping/approvals/submit",
            post(approvals::submit_approval),
        )
        .route(
            "/api/timekeeping/approvals/approve",
            post(approvals::approve_approval),
        )
        .route(
            "/api/timekeeping/approvals/reject",
            post(approvals::reject_approval),
        )
        .route(
            "/api/timekeeping/approvals/recall",
            post(approvals::recall_approval),
        )
        .route(
            "/api/timekeeping/approvals/{id}",
            get(approvals::get_approval),
        )
        .route(
            "/api/timekeeping/approvals/{id}/actions",
            get(approvals::approval_actions),
        )
        // Allocations
        .route(
            "/api/timekeeping/allocations",
            post(allocations::create_allocation).get(allocations::list_allocations),
        )
        .route(
            "/api/timekeeping/allocations/{id}",
            get(allocations::get_allocation)
                .put(allocations::update_allocation)
                .delete(allocations::deactivate_allocation),
        )
        // Rollups (actual time aggregation)
        .route(
            "/api/timekeeping/rollups/by-project",
            get(allocations::rollup_by_project),
        )
        .route(
            "/api/timekeeping/rollups/by-employee",
            get(allocations::rollup_by_employee),
        )
        .route(
            "/api/timekeeping/rollups/by-task/{project_id}",
            get(allocations::rollup_by_task),
        )
        // Exports
        .route(
            "/api/timekeeping/exports",
            post(export::create_export).get(export::list_exports),
        )
        .route(
            "/api/timekeeping/exports/{id}",
            get(export::get_export),
        )
        .with_state(state)
}

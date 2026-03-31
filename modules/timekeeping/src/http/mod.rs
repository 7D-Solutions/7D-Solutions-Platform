pub mod admin;
pub mod allocations;
pub mod approvals;
pub mod billing;
pub mod employees;
pub mod entries;
pub mod export;
pub mod projects;
pub mod tenant;

use axum::{
    routing::{get, post, put},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;
use utoipa::OpenApi;

use crate::{metrics, ops, AppState};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Timekeeping Service",
        version = "2.1.0",
        description = "Time entry, approval workflows, project billing, allocations, exports, and AR/GL integration.",
    ),
    paths(
        // Employees
        employees::create_employee,
        employees::get_employee,
        employees::list_employees,
        employees::update_employee,
        employees::deactivate_employee,
        // Projects
        projects::create_project,
        projects::get_project,
        projects::list_projects,
        projects::update_project,
        projects::deactivate_project,
        // Tasks
        projects::create_task,
        projects::list_tasks,
        projects::get_task,
        projects::update_task,
        projects::deactivate_task,
        // Entries
        entries::create_entry,
        entries::correct_entry,
        entries::void_entry,
        entries::list_entries,
        entries::entry_history,
        // Approvals
        approvals::submit_approval,
        approvals::approve_approval,
        approvals::reject_approval,
        approvals::recall_approval,
        approvals::list_approvals,
        approvals::list_pending,
        approvals::get_approval,
        approvals::approval_actions,
        // Allocations
        allocations::create_allocation,
        allocations::list_allocations,
        allocations::get_allocation,
        allocations::update_allocation,
        allocations::deactivate_allocation,
        // Rollups
        allocations::rollup_by_project,
        allocations::rollup_by_employee,
        allocations::rollup_by_task,
        // Exports
        export::create_export,
        export::get_export,
        export::list_exports,
        // Billing
        billing::create_rate,
        billing::list_rates,
        billing::create_billing_run,
    ),
    components(schemas(
        // Employees
        crate::domain::employees::models::Employee,
        crate::domain::employees::models::CreateEmployeeRequest,
        crate::domain::employees::models::UpdateEmployeeRequest,
        // Projects
        crate::domain::projects::models::Project,
        crate::domain::projects::models::Task,
        crate::domain::projects::models::CreateProjectRequest,
        crate::domain::projects::models::UpdateProjectRequest,
        crate::domain::projects::models::CreateTaskRequest,
        crate::domain::projects::models::UpdateTaskRequest,
        // Entries
        crate::domain::entries::models::TimesheetEntry,
        crate::domain::entries::models::EntryType,
        crate::domain::entries::models::CreateEntryRequest,
        crate::domain::entries::models::CorrectEntryRequest,
        crate::domain::entries::models::VoidEntryRequest,
        // Approvals
        crate::domain::approvals::models::ApprovalRequest,
        crate::domain::approvals::models::ApprovalAction,
        crate::domain::approvals::models::ApprovalStatus,
        crate::domain::approvals::models::SubmitApprovalRequest,
        crate::domain::approvals::models::ReviewApprovalRequest,
        crate::domain::approvals::models::RecallApprovalRequest,
        // Allocations
        crate::domain::allocations::models::Allocation,
        crate::domain::allocations::models::CreateAllocationRequest,
        crate::domain::allocations::models::UpdateAllocationRequest,
        crate::domain::allocations::models::ProjectRollup,
        crate::domain::allocations::models::EmployeeRollup,
        crate::domain::allocations::models::TaskRollup,
        // Exports
        crate::domain::export::models::ExportRun,
        crate::domain::export::models::ExportStatus,
        crate::domain::export::models::ExportArtifact,
        crate::domain::export::models::CreateExportRunRequest,
        // Billing
        crate::domain::billing::models::BillingRate,
        crate::domain::billing::models::BillingRun,
        crate::domain::billing::models::BillingLineItem,
        crate::domain::billing::models::BillingRunResult,
        crate::domain::billing::models::CreateBillingRateRequest,
        crate::domain::billing::models::CreateBillingRunRequest,
        // Platform
        platform_http_contracts::ApiError,
    )),
    security(("bearer" = [])),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

/// Build the Timekeeping HTTP router with all endpoints.
///
/// Mutation routes (POST / PUT / DELETE) require the `timekeeping.mutate`
/// permission in the caller's JWT.  Read routes are unenforced at this stage.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        // Employees — write
        .route(
            "/api/timekeeping/employees",
            post(employees::create_employee),
        )
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
        .route(
            "/api/timekeeping/entries/correct",
            post(entries::correct_entry),
        )
        .route("/api/timekeeping/entries/void", post(entries::void_entry))
        // Approvals — write
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
        // Allocations — write
        .route(
            "/api/timekeeping/allocations",
            post(allocations::create_allocation),
        )
        .route(
            "/api/timekeeping/allocations/{id}",
            put(allocations::update_allocation).delete(allocations::deactivate_allocation),
        )
        // Exports — write
        .route("/api/timekeeping/exports", post(export::create_export))
        // Billing rates + billing runs — write
        .route("/api/timekeeping/rates", post(billing::create_rate))
        .route(
            "/api/timekeeping/billing-runs",
            post(billing::create_billing_run),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::TIMEKEEPING_MUTATE,
        ]))
        .with_state(state.clone());

    let reads = Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(ops::health::health))
        .route("/api/ready", get(ops::ready::ready))
        .route("/api/version", get(ops::version::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Employees — read
        .route("/api/timekeeping/employees", get(employees::list_employees))
        .route(
            "/api/timekeeping/employees/{id}",
            get(employees::get_employee),
        )
        // Projects — read
        .route("/api/timekeeping/projects", get(projects::list_projects))
        .route("/api/timekeeping/projects/{id}", get(projects::get_project))
        .route(
            "/api/timekeeping/projects/{project_id}/tasks",
            get(projects::list_tasks),
        )
        .route("/api/timekeeping/tasks/{id}", get(projects::get_task))
        // Entries — read
        .route("/api/timekeeping/entries", get(entries::list_entries))
        .route(
            "/api/timekeeping/entries/{entry_id}/history",
            get(entries::entry_history),
        )
        // Approvals — read
        .route("/api/timekeeping/approvals", get(approvals::list_approvals))
        .route(
            "/api/timekeeping/approvals/pending",
            get(approvals::list_pending),
        )
        .route(
            "/api/timekeeping/approvals/{id}",
            get(approvals::get_approval),
        )
        .route(
            "/api/timekeeping/approvals/{id}/actions",
            get(approvals::approval_actions),
        )
        // Allocations — read
        .route(
            "/api/timekeeping/allocations",
            get(allocations::list_allocations),
        )
        .route(
            "/api/timekeeping/allocations/{id}",
            get(allocations::get_allocation),
        )
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
        // Exports — read
        .route("/api/timekeeping/exports", get(export::list_exports))
        .route("/api/timekeeping/exports/{id}", get(export::get_export))
        // Billing rates — read
        .route("/api/timekeeping/rates", get(billing::list_rates))
        .with_state(state);

    Router::new().merge(mutations).merge(reads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid_json() {
        let spec = ApiDoc::openapi();
        let json =
            serde_json::to_string_pretty(&spec).expect("OpenAPI spec must serialize to JSON");
        assert!(json.contains("\"openapi\""), "must contain openapi version");
        assert!(
            json.contains("/api/timekeeping/employees"),
            "must contain employees path"
        );
        assert!(
            json.contains("/api/timekeeping/projects"),
            "must contain projects path"
        );
        assert!(
            json.contains("/api/timekeeping/entries"),
            "must contain entries path"
        );
        assert!(
            json.contains("/api/timekeeping/approvals"),
            "must contain approvals path"
        );
        assert!(
            json.contains("/api/timekeeping/allocations"),
            "must contain allocations path"
        );
        assert!(
            json.contains("/api/timekeeping/exports"),
            "must contain exports path"
        );
        assert!(
            json.contains("/api/timekeeping/rates"),
            "must contain rates path"
        );
        assert!(
            json.contains("/api/timekeeping/billing-runs"),
            "must contain billing-runs path"
        );
        assert!(
            json.contains("/api/timekeeping/rollups/by-project"),
            "must contain rollups path"
        );
        assert!(
            json.contains("\"Employee\""),
            "must have Employee schema"
        );
        assert!(
            json.contains("\"TimesheetEntry\""),
            "must have TimesheetEntry schema"
        );
        assert!(
            json.contains("\"ApprovalRequest\""),
            "must have ApprovalRequest schema"
        );
        assert!(
            json.contains("\"ApiError\""),
            "must have ApiError schema"
        );
        assert!(
            json.contains("\"BillingRate\""),
            "must have BillingRate schema"
        );
    }
}

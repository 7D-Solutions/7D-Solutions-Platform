pub mod activity;
pub mod complaints;
pub mod sweep;
pub mod taxonomy;
pub mod tenant;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Customer Complaints Service",
        version = "0.1.0",
        description = "Customer complaint lifecycle: intake, triage, investigation, resolution, and closure.",
    ),
    paths(
        // Complaints
        complaints::create_complaint,
        complaints::get_complaint,
        complaints::list_complaints,
        complaints::update_complaint,
        complaints::triage_complaint,
        complaints::start_investigation,
        complaints::respond_complaint,
        complaints::close_complaint,
        complaints::cancel_complaint,
        complaints::assign_complaint,
        // Activity & Resolution
        activity::add_note,
        activity::add_customer_communication,
        activity::list_activity_log,
        activity::create_resolution,
        activity::get_resolution,
        // Taxonomy & Labels
        taxonomy::list_categories,
        taxonomy::create_category,
        taxonomy::update_category,
        taxonomy::list_status_labels,
        taxonomy::set_status_label,
        taxonomy::list_severity_labels,
        taxonomy::set_severity_label,
        taxonomy::list_source_labels,
        taxonomy::set_source_label,
        // Admin
        sweep::sweep_overdue,
    ),
    components(schemas(
        crate::domain::models::Complaint,
        crate::domain::models::ComplaintDetail,
        crate::domain::models::CreateComplaintRequest,
        crate::domain::models::UpdateComplaintRequest,
        crate::domain::models::TriageComplaintRequest,
        crate::domain::models::StartInvestigationRequest,
        crate::domain::models::RespondComplaintRequest,
        crate::domain::models::CloseComplaintRequest,
        crate::domain::models::CancelComplaintRequest,
        crate::domain::models::AssignComplaintRequest,
        crate::domain::models::ComplaintStatus,
        crate::domain::models::ComplaintSeverity,
        crate::domain::models::ComplaintSource,
        crate::domain::models::ComplaintOutcome,
        crate::domain::models::CustomerAcceptance,
        crate::domain::models::ActivityType,
        crate::domain::models::ComplaintActivityLog,
        crate::domain::models::CreateActivityLogRequest,
        crate::domain::models::ComplaintResolution,
        crate::domain::models::CreateResolutionRequest,
        crate::domain::models::ComplaintCategoryCode,
        crate::domain::models::CreateCategoryCodeRequest,
        crate::domain::models::UpdateCategoryCodeRequest,
        crate::domain::models::CcStatusLabel,
        crate::domain::models::CcSeverityLabel,
        crate::domain::models::CcSourceLabel,
        crate::domain::models::UpsertLabelRequest,
        platform_http_contracts::ApiError,
        crate::http::sweep::SweepOverdueResponse,
    )),
    tags(
        (name = "Complaints", description = "Complaint lifecycle management"),
        (name = "Activity", description = "Activity log and resolution recording"),
        (name = "Taxonomy", description = "Per-tenant category codes and display labels"),
        (name = "Admin", description = "Administrative operations"),
    ),
    security(("bearer" = [])),
)]
pub struct ApiDoc;

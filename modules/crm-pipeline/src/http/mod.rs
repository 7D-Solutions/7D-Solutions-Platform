pub mod activities;
pub mod contacts;
pub mod labels;
pub mod leads;
pub mod opportunities;
pub mod stages;
pub mod tenant;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "CRM Pipeline Service",
        version = "0.1.0",
        description = "CRM Pipeline: leads, opportunities, pipeline stages, activities, and sales handoff.",
    ),
    paths(
        // Leads
        leads::create_lead,
        leads::get_lead,
        leads::list_leads,
        leads::update_lead,
        leads::mark_contacted,
        leads::mark_qualifying,
        leads::mark_qualified,
        leads::convert_lead,
        leads::disqualify_lead,
        leads::mark_dead,
        // Opportunities
        opportunities::create_opportunity,
        opportunities::get_opportunity,
        opportunities::list_opportunities,
        opportunities::update_opportunity,
        opportunities::advance_stage,
        opportunities::close_won,
        opportunities::close_lost,
        opportunities::stage_history,
        opportunities::pipeline_summary,
        // Pipeline Stages
        stages::list_stages,
        stages::create_stage,
        stages::update_stage,
        stages::deactivate_stage,
        stages::reorder_stages,
        // Activities
        activities::log_activity,
        activities::get_activity,
        activities::list_activities,
        activities::complete_activity,
        activities::update_activity,
        activities::list_activity_types,
        activities::create_activity_type,
        activities::update_activity_type,
        // Contact Roles
        contacts::get_contact_attributes,
        contacts::set_contact_attributes,
        // Labels
        labels::list_status_labels,
        labels::list_source_labels,
        labels::list_type_labels,
        labels::list_priority_labels,
    ),
    components(schemas(
        crate::domain::leads::Lead,
        crate::domain::leads::CreateLeadRequest,
        crate::domain::leads::UpdateLeadRequest,
        crate::domain::leads::ConvertLeadRequest,
        crate::domain::leads::DisqualifyLeadRequest,
        crate::domain::leads::ConvertLeadResponse,
        crate::domain::leads::ListLeadsQuery,
        crate::domain::opportunities::Opportunity,
        crate::domain::opportunities::OpportunityStageHistory,
        crate::domain::opportunities::OpportunityDetail,
        crate::domain::opportunities::PipelineSummaryItem,
        crate::domain::opportunities::CreateOpportunityRequest,
        crate::domain::opportunities::UpdateOpportunityRequest,
        crate::domain::opportunities::AdvanceStageRequest,
        crate::domain::opportunities::CloseWonRequest,
        crate::domain::opportunities::CloseLostRequest,
        crate::domain::opportunities::ListOpportunitiesQuery,
        crate::domain::pipeline_stages::PipelineStage,
        crate::domain::pipeline_stages::CreateStageRequest,
        crate::domain::pipeline_stages::UpdateStageRequest,
        crate::domain::pipeline_stages::ReorderStagesRequest,
        crate::domain::pipeline_stages::StageReorderItem,
        crate::domain::activities::Activity,
        crate::domain::activities::CreateActivityRequest,
        crate::domain::activities::UpdateActivityRequest,
        crate::domain::activities::ListActivitiesQuery,
        crate::domain::activity_types::ActivityType,
        crate::domain::activity_types::CreateActivityTypeRequest,
        crate::domain::activity_types::UpdateActivityTypeRequest,
        crate::domain::contact_role_attributes::ContactRoleAttributes,
        crate::domain::contact_role_attributes::UpsertContactRoleRequest,
        crate::domain::labels::Label,
        platform_http_contracts::ApiError,
    )),
    tags(
        (name = "Leads", description = "Lead lifecycle management"),
        (name = "Opportunities", description = "Opportunity pipeline management"),
        (name = "PipelineStages", description = "Tenant pipeline stage configuration"),
        (name = "Activities", description = "Sales activity logging"),
        (name = "ContactRoles", description = "CRM contact role attributes"),
        (name = "Labels", description = "Per-tenant display labels"),
    ),
    security(("bearer" = [])),
)]
pub struct ApiDoc;

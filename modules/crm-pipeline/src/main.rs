use axum::{routing::{get, post, put}, Json, Router};
use std::sync::Arc;
use utoipa::OpenApi;

use crm_pipeline_rs::{http, metrics, AppState};
use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let crm_metrics = Arc::new(
                metrics::CrmPipelineMetrics::new().expect("CRM: failed to create metrics"),
            );

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: crm_metrics,
            });

            if let Ok(bus) = ctx.bus_arc() {
                crm_pipeline_rs::consumers::party_deactivated::start_party_deactivated_consumer(
                    bus.clone(), ctx.pool().clone(),
                );
                crm_pipeline_rs::consumers::contact_deactivated::start_contact_deactivated_consumer(
                    bus.clone(), ctx.pool().clone(),
                );
                crm_pipeline_rs::consumers::order_booked::start_order_booked_consumer(
                    bus.clone(), ctx.pool().clone(),
                );
                crm_pipeline_rs::consumers::customer_created::start_customer_created_consumer(
                    bus, ctx.pool().clone(),
                );
                tracing::info!("CRM Pipeline: consumers started");
            }

            let crm_mutations = Router::new()
                // Leads
                .route("/api/crm-pipeline/leads", post(http::leads::create_lead))
                .route("/api/crm-pipeline/leads/{id}", put(http::leads::update_lead))
                .route("/api/crm-pipeline/leads/{id}/contact", post(http::leads::mark_contacted))
                .route("/api/crm-pipeline/leads/{id}/qualify", post(http::leads::mark_qualifying))
                .route("/api/crm-pipeline/leads/{id}/mark-qualified", post(http::leads::mark_qualified))
                .route("/api/crm-pipeline/leads/{id}/convert", post(http::leads::convert_lead))
                .route("/api/crm-pipeline/leads/{id}/disqualify", post(http::leads::disqualify_lead))
                .route("/api/crm-pipeline/leads/{id}/mark-dead", post(http::leads::mark_dead))
                // Opportunities
                .route("/api/crm-pipeline/opportunities", post(http::opportunities::create_opportunity))
                .route("/api/crm-pipeline/opportunities/{id}", put(http::opportunities::update_opportunity))
                .route("/api/crm-pipeline/opportunities/{id}/advance-stage", post(http::opportunities::advance_stage))
                .route("/api/crm-pipeline/opportunities/{id}/close-won", post(http::opportunities::close_won))
                .route("/api/crm-pipeline/opportunities/{id}/close-lost", post(http::opportunities::close_lost))
                // Pipeline stage config
                .route("/api/crm-pipeline/stages", post(http::stages::create_stage))
                .route("/api/crm-pipeline/stages/{code}", put(http::stages::update_stage))
                .route("/api/crm-pipeline/stages/{code}/deactivate", post(http::stages::deactivate_stage))
                .route("/api/crm-pipeline/stages/reorder", post(http::stages::reorder_stages))
                // Activities
                .route("/api/crm-pipeline/activities", post(http::activities::log_activity))
                .route("/api/crm-pipeline/activities/{id}/complete", post(http::activities::complete_activity))
                .route("/api/crm-pipeline/activities/{id}", put(http::activities::update_activity))
                .route("/api/crm-pipeline/activity-types", post(http::activities::create_activity_type))
                .route("/api/crm-pipeline/activity-types/{code}", put(http::activities::update_activity_type))
                // Contact roles
                .route("/api/crm-pipeline/contacts/{party_contact_id}/attributes", put(http::contacts::set_contact_attributes))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CRM_PIPELINE_MUTATE]))
                .with_state(app_state.clone());

            let crm_reads = Router::new()
                .route("/api/crm-pipeline/leads", get(http::leads::list_leads))
                .route("/api/crm-pipeline/leads/{id}", get(http::leads::get_lead))
                .route("/api/crm-pipeline/opportunities", get(http::opportunities::list_opportunities))
                .route("/api/crm-pipeline/opportunities/{id}", get(http::opportunities::get_opportunity))
                .route("/api/crm-pipeline/opportunities/{id}/stage-history", get(http::opportunities::stage_history))
                .route("/api/crm-pipeline/pipeline/summary", get(http::opportunities::pipeline_summary))
                .route("/api/crm-pipeline/stages", get(http::stages::list_stages))
                .route("/api/crm-pipeline/activities", get(http::activities::list_activities))
                .route("/api/crm-pipeline/activities/{id}", get(http::activities::get_activity))
                .route("/api/crm-pipeline/activity-types", get(http::activities::list_activity_types))
                .route("/api/crm-pipeline/contacts/{party_contact_id}/attributes", get(http::contacts::get_contact_attributes))
                .route("/api/crm-pipeline/status-labels", get(http::labels::list_status_labels))
                .route("/api/crm-pipeline/source-labels", get(http::labels::list_source_labels))
                .route("/api/crm-pipeline/type-labels", get(http::labels::list_type_labels))
                .route("/api/crm-pipeline/priority-labels", get(http::labels::list_priority_labels))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CRM_PIPELINE_READ]))
                .with_state(app_state);

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(crm_reads)
                .merge(crm_mutations)
        })
        .run()
        .await
        .expect("crm-pipeline module failed");
}

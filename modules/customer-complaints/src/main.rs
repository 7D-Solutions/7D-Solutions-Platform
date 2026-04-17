use axum::{routing::{get, post, put}, Json, Router};
use std::sync::Arc;
use utoipa::OpenApi;

use customer_complaints_rs::{http, metrics, AppState};
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
            let cc_metrics =
                Arc::new(metrics::CcMetrics::new().expect("CC: failed to create metrics"));

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: cc_metrics,
            });

            // Read routes — require customer_complaints.read
            let cc_reads = Router::new()
                .route("/api/customer-complaints/complaints", get(http::complaints::list_complaints))
                .route("/api/customer-complaints/complaints/{id}", get(http::complaints::get_complaint))
                .route("/api/customer-complaints/complaints/{id}/activity-log", get(http::activity::list_activity_log))
                .route("/api/customer-complaints/complaints/{id}/resolution", get(http::activity::get_resolution))
                .route("/api/customer-complaints/categories", get(http::taxonomy::list_categories))
                .route("/api/customer-complaints/status-labels", get(http::taxonomy::list_status_labels))
                .route("/api/customer-complaints/severity-labels", get(http::taxonomy::list_severity_labels))
                .route("/api/customer-complaints/source-labels", get(http::taxonomy::list_source_labels))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CUSTOMER_COMPLAINTS_READ]))
                .with_state(app_state.clone());

            // General mutation routes — require customer_complaints.mutate
            let cc_mutations = Router::new()
                .route("/api/customer-complaints/complaints", post(http::complaints::create_complaint))
                .route("/api/customer-complaints/complaints/{id}", put(http::complaints::update_complaint))
                .route("/api/customer-complaints/complaints/{id}/start-investigation", post(http::complaints::start_investigation))
                .route("/api/customer-complaints/complaints/{id}/respond", post(http::complaints::respond_complaint))
                .route("/api/customer-complaints/complaints/{id}/assign", post(http::complaints::assign_complaint))
                .route("/api/customer-complaints/complaints/{id}/notes", post(http::activity::add_note))
                .route("/api/customer-complaints/complaints/{id}/customer-communication", post(http::activity::add_customer_communication))
                .route("/api/customer-complaints/complaints/{id}/resolution", post(http::activity::create_resolution))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CUSTOMER_COMPLAINTS_MUTATE]))
                .with_state(app_state.clone());

            // Triage — requires complaint.triage permission
            let cc_triage = Router::new()
                .route("/api/customer-complaints/complaints/{id}/triage", post(http::complaints::triage_complaint))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CC_COMPLAINT_TRIAGE]))
                .with_state(app_state.clone());

            // Close — requires complaint.close permission
            let cc_close = Router::new()
                .route("/api/customer-complaints/complaints/{id}/close", post(http::complaints::close_complaint))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CC_COMPLAINT_CLOSE]))
                .with_state(app_state.clone());

            // Cancel — requires complaint.cancel permission
            let cc_cancel = Router::new()
                .route("/api/customer-complaints/complaints/{id}/cancel", post(http::complaints::cancel_complaint))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CC_COMPLAINT_CANCEL]))
                .with_state(app_state.clone());

            // Category management — requires category.manage permission
            let cc_category_manage = Router::new()
                .route("/api/customer-complaints/categories", post(http::taxonomy::create_category))
                .route("/api/customer-complaints/categories/{code}", put(http::taxonomy::update_category))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CC_CATEGORY_MANAGE]))
                .with_state(app_state.clone());

            // Label editing — requires labels.edit permission
            let cc_labels_edit = Router::new()
                .route("/api/customer-complaints/status-labels/{canonical}", put(http::taxonomy::set_status_label))
                .route("/api/customer-complaints/severity-labels/{canonical}", put(http::taxonomy::set_severity_label))
                .route("/api/customer-complaints/source-labels/{canonical}", put(http::taxonomy::set_source_label))
                .route_layer(RequirePermissionsLayer::new(&[permissions::CC_LABELS_EDIT]))
                .with_state(app_state);

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(cc_reads)
                .merge(cc_mutations)
                .merge(cc_triage)
                .merge(cc_close)
                .merge(cc_cancel)
                .merge(cc_category_manage)
                .merge(cc_labels_edit)
        })
        .run()
        .await
        .expect("customer-complaints module failed");
}

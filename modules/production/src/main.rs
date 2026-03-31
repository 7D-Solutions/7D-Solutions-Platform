use axum::{routing::get, Json};
use std::sync::Arc;
use utoipa::OpenApi;

use production_rs::domain::component_issue::{RequestComponentIssueRequest, ComponentIssueItemInput};
use production_rs::domain::downtime::{WorkcenterDowntime, StartDowntimeRequest, EndDowntimeRequest};
use production_rs::domain::fg_receipt::RequestFgReceiptRequest;
use production_rs::domain::operations::OperationInstance;
use production_rs::domain::routings::{
    RoutingTemplate, RoutingStep, CreateRoutingRequest, UpdateRoutingRequest,
    AddRoutingStepRequest,
};
use production_rs::domain::time_entries::{TimeEntry, StartTimerRequest, StopTimerRequest, ManualEntryRequest};
use production_rs::domain::work_orders::{WorkOrder, WorkOrderStatus, CreateWorkOrderRequest};
use production_rs::domain::workcenters::{Workcenter, CreateWorkcenterRequest, UpdateWorkcenterRequest};
use production_rs::http::pagination::PaginationQuery;
use production_rs::http::routings::ItemDateQuery;
use production_rs::{http, metrics, AppState};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use platform_sdk::ModuleBuilder;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Production Service",
        version = "2.1.0",
        description = "Production execution: work orders, operations, workcenters, routing, \
                        component issue/receipt workflows, time entries, downtime tracking.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permissions: `PRODUCTION_READ` for queries, \
                        `PRODUCTION_MUTATE` for writes.\n\n\
                        **Events:** All state mutations are published to the outbox table for \
                        downstream consumers.",
    ),
    paths(
        production_rs::http::workcenters::create_workcenter,
        production_rs::http::workcenters::get_workcenter,
        production_rs::http::workcenters::list_workcenters,
        production_rs::http::workcenters::update_workcenter,
        production_rs::http::workcenters::deactivate_workcenter,
        production_rs::http::work_orders::create_work_order,
        production_rs::http::work_orders::release_work_order,
        production_rs::http::work_orders::close_work_order,
        production_rs::http::work_orders::get_work_order,
        production_rs::http::operations::initialize_operations,
        production_rs::http::operations::start_operation,
        production_rs::http::operations::complete_operation,
        production_rs::http::operations::list_operations,
        production_rs::http::time_entries::start_timer,
        production_rs::http::time_entries::stop_timer,
        production_rs::http::time_entries::manual_entry,
        production_rs::http::time_entries::list_time_entries,
        production_rs::http::routings::create_routing,
        production_rs::http::routings::get_routing,
        production_rs::http::routings::list_routings,
        production_rs::http::routings::find_routings_by_item,
        production_rs::http::routings::update_routing,
        production_rs::http::routings::release_routing,
        production_rs::http::routings::add_routing_step,
        production_rs::http::routings::list_routing_steps,
        production_rs::http::downtime::start_downtime,
        production_rs::http::downtime::end_downtime,
        production_rs::http::downtime::list_active_downtime,
        production_rs::http::downtime::list_workcenter_downtime,
        production_rs::http::component_issue::post_component_issue,
        production_rs::http::fg_receipt::post_fg_receipt,
    ),
    components(schemas(
        Workcenter, CreateWorkcenterRequest, UpdateWorkcenterRequest,
        WorkOrder, WorkOrderStatus, CreateWorkOrderRequest,
        OperationInstance,
        TimeEntry, StartTimerRequest, StopTimerRequest, ManualEntryRequest,
        WorkcenterDowntime, StartDowntimeRequest, EndDowntimeRequest,
        RoutingTemplate, RoutingStep, CreateRoutingRequest, UpdateRoutingRequest,
        AddRoutingStepRequest,
        RequestComponentIssueRequest, ComponentIssueItemInput,
        RequestFgReceiptRequest,
        ApiError, PaginatedResponse<Workcenter>, PaginatedResponse<RoutingTemplate>,
        PaginatedResponse<WorkcenterDowntime>, PaginationMeta, PaginationQuery,
        ItemDateQuery,
    )),
    security(
        ("bearer" = [])
    ),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

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

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let prod_metrics = Arc::new(
                metrics::ProductionMetrics::new().expect("Production: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: prod_metrics,
            });
            http::router(app_state)
                .route("/api/openapi.json", get(openapi_json))
        })
        .run()
        .await
        .expect("production module failed");
}

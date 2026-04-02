//! Utility binary that prints the Maintenance OpenAPI spec as JSON to stdout.
//! No database or NATS connection required.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Maintenance Service",
        version = "2.2.0",
        description = "Maintenance management: work orders, preventive plans, meters, calibration, \
                        downtime tracking, and labor management.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims.\n\
                        Permissions: MAINTENANCE_READ for queries, MAINTENANCE_MUTATE for writes."
    ),
    paths(
        // Assets
        maintenance_rs::http::assets::create_asset,
        maintenance_rs::http::assets::list_assets,
        maintenance_rs::http::assets::get_asset,
        maintenance_rs::http::assets::update_asset,
        // Calibration
        maintenance_rs::http::calibration_events::record_calibration_event,
        maintenance_rs::http::calibration_events::get_calibration_status,
        // Downtime
        maintenance_rs::http::downtime::create_downtime,
        maintenance_rs::http::downtime::list_downtime,
        maintenance_rs::http::downtime::get_downtime,
        maintenance_rs::http::downtime::list_asset_downtime,
        // Health
        maintenance_rs::http::health::health,
        maintenance_rs::http::health::ready,
        maintenance_rs::http::health::version,
        // Meters
        maintenance_rs::http::meters::create_meter_type,
        maintenance_rs::http::meters::list_meter_types,
        maintenance_rs::http::meters::record_reading,
        maintenance_rs::http::meters::list_readings,
        // Plans
        maintenance_rs::http::plans::create_plan,
        maintenance_rs::http::plans::list_plans,
        maintenance_rs::http::plans::get_plan,
        maintenance_rs::http::plans::update_plan,
        maintenance_rs::http::plans::assign_plan,
        maintenance_rs::http::plans::list_assignments,
        // Work Orders
        maintenance_rs::http::work_orders::create_work_order,
        maintenance_rs::http::work_orders::list_work_orders,
        maintenance_rs::http::work_orders::get_work_order,
        maintenance_rs::http::work_orders::transition_work_order,
        // Work Order Labor
        maintenance_rs::http::work_order_labor::add_labor,
        maintenance_rs::http::work_order_labor::list_labor,
        maintenance_rs::http::work_order_labor::remove_labor,
        // Work Order Parts
        maintenance_rs::http::work_order_parts::add_part,
        maintenance_rs::http::work_order_parts::list_parts,
        maintenance_rs::http::work_order_parts::remove_part,
    ),
    components(schemas(
        maintenance_rs::domain::assets::Asset,
        maintenance_rs::domain::assets::CreateAssetRequest,
        maintenance_rs::domain::assets::UpdateAssetRequest,
        maintenance_rs::domain::calibration_events::CalibrationEvent,
        maintenance_rs::domain::calibration_events::CalibrationStatus,
        maintenance_rs::domain::calibration_events::CalibrationStatusResponse,
        maintenance_rs::domain::calibration_events::RecordCalibrationRequest,
        maintenance_rs::domain::downtime::DowntimeEvent,
        maintenance_rs::domain::downtime::CreateDowntimeRequest,
        maintenance_rs::domain::meters::MeterType,
        maintenance_rs::domain::meters::MeterReading,
        maintenance_rs::domain::meters::CreateMeterTypeRequest,
        maintenance_rs::domain::meters::RecordReadingRequest,
        maintenance_rs::domain::plans::MaintenancePlan,
        maintenance_rs::domain::plans::PlanAssignment,
        maintenance_rs::domain::plans::CreatePlanRequest,
        maintenance_rs::domain::plans::UpdatePlanRequest,
        maintenance_rs::domain::plans::AssignPlanRequest,
        maintenance_rs::domain::work_orders::WorkOrder,
        maintenance_rs::domain::work_orders::CreateWorkOrderRequest,
        maintenance_rs::domain::work_orders::TransitionRequest,
        maintenance_rs::domain::work_orders::WoLabor,
        maintenance_rs::domain::work_orders::AddLaborRequest,
        maintenance_rs::domain::work_orders::WoPart,
        maintenance_rs::domain::work_orders::AddPartRequest,
        platform_http_contracts::ApiError,
    )),
    security(("bearer" = [])),
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

fn main() {
    let spec = ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}

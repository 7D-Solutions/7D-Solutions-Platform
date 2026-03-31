pub mod health;
pub mod inspection_routes;
pub mod tenant;

use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Quality Inspection Service",
        version = "2.0.0",
        description = "Receiving inspection, in-process inspection, final inspection, and disposition tracking.",
    ),
    paths(
        inspection_routes::post_inspection_plan,
        inspection_routes::get_inspection_plan,
        inspection_routes::post_activate_plan,
        inspection_routes::post_receiving_inspection,
        inspection_routes::get_inspection,
        inspection_routes::post_hold_inspection,
        inspection_routes::post_release_inspection,
        inspection_routes::post_accept_inspection,
        inspection_routes::post_reject_inspection,
        inspection_routes::post_in_process_inspection,
        inspection_routes::post_final_inspection,
        inspection_routes::get_inspections_by_part_rev,
        inspection_routes::get_inspections_by_receipt,
        inspection_routes::get_inspections_by_wo,
        inspection_routes::get_inspections_by_lot,
    ),
    components(schemas(
        crate::domain::models::InspectionPlan,
        crate::domain::models::Inspection,
        crate::domain::models::Characteristic,
        crate::domain::models::CreateInspectionPlanRequest,
        crate::domain::models::CreateReceivingInspectionRequest,
        crate::domain::models::CreateInProcessInspectionRequest,
        crate::domain::models::CreateFinalInspectionRequest,
        crate::domain::models::DispositionTransitionRequest,
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<crate::domain::models::Inspection>,
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

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

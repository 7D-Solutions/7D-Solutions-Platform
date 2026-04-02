pub mod definitions;
pub mod instances;
pub mod tenant;

use axum::Json;
use utoipa::OpenApi;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Workflow Service",
        version = "2.1.0",
        description = "Durable workflow engine: definitions, instances, step routing, and event-driven execution.",
    ),
    paths(
        definitions::create_definition,
        definitions::list_definitions,
        definitions::get_definition,
        instances::start_instance,
        instances::advance_instance,
        instances::get_instance,
        instances::list_instances,
        instances::list_transitions,
    ),
    components(schemas(
        crate::domain::definitions::WorkflowDefinition,
        crate::domain::definitions::CreateDefinitionRequest,
        crate::domain::instances::WorkflowInstance,
        crate::domain::instances::WorkflowTransition,
        crate::domain::instances::StartInstanceRequest,
        crate::domain::instances::AdvanceInstanceRequest,
        crate::domain::types::InstanceStatus,
        instances::AdvanceResponse,
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<crate::domain::definitions::WorkflowDefinition>,
        platform_http_contracts::PaginatedResponse<crate::domain::instances::WorkflowInstance>,
        platform_http_contracts::PaginationMeta,
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

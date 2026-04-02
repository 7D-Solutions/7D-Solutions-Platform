pub mod items;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Smoke Test Service",
        version = "0.1.0",
        description = "Minimal greenfield module for plug-and-play verification.",
    ),
    paths(
        items::create_item,
        items::get_item,
        items::list_items,
        items::update_item,
        items::delete_item,
    ),
    components(schemas(
        items::CreateItemRequest,
        items::UpdateItemRequest,
        items::ItemResponse,
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<items::ItemResponse>,
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

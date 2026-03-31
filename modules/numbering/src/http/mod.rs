pub mod allocate;
pub mod confirm;
pub mod health;
pub mod policy;
pub mod tenant;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Numbering Service",
        version = "2.1.0",
        description = "Tenant-scoped, idempotent, atomic sequence allocation.",
    ),
    paths(
        allocate::allocate,
        confirm::confirm,
        policy::upsert_policy,
        policy::get_policy,
    ),
    components(schemas(
        allocate::AllocateRequest, allocate::AllocateResponse,
        confirm::ConfirmRequest, confirm::ConfirmResponse,
        policy::UpsertPolicyRequest, policy::PolicyResponse,
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

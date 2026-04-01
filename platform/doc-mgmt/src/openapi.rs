use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Doc-Mgmt Service",
        version = "1.0.1",
        description = "Document management: core doc model, revision tracking, lifecycle \
                        (draft → released), distribution control, retention policies, \
                        legal holds, and template rendering.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims.\n\
                        Permissions: `doc_mgmt.read` for queries, `doc_mgmt.mutate` for writes."
    ),
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

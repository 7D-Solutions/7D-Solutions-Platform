pub mod admin;
pub mod auth;
pub mod docs;
pub mod protected;
pub mod status;
pub mod tenant;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Customer Portal",
        version = "2.1.0",
        description = "External customer auth boundary with RS256 JWT, document visibility, and status feed.",
    ),
    paths(
        auth::login,
        auth::refresh,
        auth::logout,
        admin::invite_user,
        status::create_status_card,
        status::list_status_cards,
        status::acknowledge,
        status::link_document,
        docs::list_documents,
        protected::me,
        protected::party_guard_probe,
    ),
    components(schemas(
        auth::LoginRequest, auth::RefreshRequest, auth::LogoutRequest, auth::AuthResponse,
        admin::InviteUserRequest, admin::InviteUserResponse,
        status::CreateStatusCardRequest, status::AcknowledgeRequest,
        status::StatusCard, status::LinkDocumentRequest,
        docs::PortalDocumentView,
        protected::MeResponse,
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

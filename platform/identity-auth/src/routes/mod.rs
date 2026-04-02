pub mod auth;
pub mod health;
pub mod jwks;
pub mod metrics;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Identity & Auth Service",
        version = "1.8.0",
        description = "JWT authentication, RBAC, session management, password reset, and \
                        Separation of Duties policy enforcement.\n\n\
                        **Public endpoints** (register, login, forgot-password, reset-password) \
                        do not require a Bearer token.\n\
                        **All other endpoints** require `Authorization: Bearer <JWT>`."
    ),
    paths(
        // Auth
        crate::auth::handlers::register,
        crate::auth::handlers::login,
        crate::auth::session::refresh,
        crate::auth::session::logout,
        // Password reset
        crate::auth::handlers_password_reset::forgot_password,
        crate::auth::handlers_password_reset::reset_password,
        // Users
        crate::auth::handlers::get_user_by_email,
        // RBAC
        crate::auth::handlers::list_roles,
        crate::auth::handlers::list_permissions,
        // Lifecycle
        crate::auth::handlers::record_access_review,
        crate::auth::handlers::get_user_lifecycle_timeline,
        // SoD
        crate::auth::handlers::upsert_sod_policy,
        crate::auth::handlers::evaluate_sod,
        crate::auth::handlers::list_sod_policies,
        crate::auth::handlers::delete_sod_policy,
    ),
    components(schemas(
        crate::auth::handlers::RegisterReq,
        crate::auth::handlers::LoginReq,
        crate::auth::handlers::TokenResponse,
        crate::auth::handlers::OkResponse,
        crate::auth::handlers::AccessReviewReq,
        crate::auth::handlers::SodPolicyUpsertReq,
        crate::auth::handlers::SodEvaluateReq,
        crate::auth::handlers::UserLookupResponse,
        crate::auth::session::RefreshReq,
        crate::auth::session::LogoutReq,
        crate::auth::handlers_password_reset::ForgotPasswordRequest,
        crate::auth::handlers_password_reset::ResetPasswordRequest,
        crate::auth::handlers_password_reset::GenericOkResponse,
        crate::db::rbac::Role,
        crate::db::rbac::Permission,
        crate::db::sod::SodPolicy,
        crate::db::sod::SodPolicyUpsertResult,
        crate::db::sod::SodDecisionResult,
        crate::db::user_lifecycle_audit::LifecycleTimelineEntry,
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

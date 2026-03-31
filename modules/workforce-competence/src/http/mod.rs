pub mod handlers;
pub mod health;
pub mod tenant;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Workforce Competence Service",
        version = "2.1.0",
        description = "Competence registry: skills, certifications, training records, acceptance authorities, and authorization queries.",
    ),
    paths(
        handlers::post_artifact,
        handlers::get_artifact,
        handlers::post_assignment,
        handlers::get_authorization,
        handlers::post_grant_authority,
        handlers::post_revoke_authority,
        handlers::get_acceptance_authority_check,
    ),
    components(schemas(
        crate::domain::models::ArtifactType,
        crate::domain::models::CompetenceArtifact,
        crate::domain::models::RegisterArtifactRequest,
        crate::domain::models::OperatorCompetence,
        crate::domain::models::AssignCompetenceRequest,
        crate::domain::models::AuthorizationResult,
        crate::domain::acceptance_authority::AcceptanceAuthority,
        crate::domain::acceptance_authority::GrantAuthorityRequest,
        crate::domain::acceptance_authority::RevokeAuthorityRequest,
        crate::domain::acceptance_authority::AcceptanceAuthorityResult,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid_json() {
        let spec = ApiDoc::openapi();
        let json = serde_json::to_string_pretty(&spec).expect("OpenAPI spec must serialize to JSON");
        assert!(json.contains("\"openapi\""), "must contain openapi version");
        assert!(json.contains("/api/workforce-competence/artifacts"), "must contain artifacts path");
        assert!(json.contains("/api/workforce-competence/assignments"), "must contain assignments path");
        assert!(json.contains("/api/workforce-competence/authorization"), "must contain authorization path");
        assert!(json.contains("/api/workforce-competence/acceptance-authorities"), "must contain acceptance-authorities path");
        assert!(json.contains("/api/workforce-competence/acceptance-authority-check"), "must contain authority-check path");
        assert!(json.contains("\"CompetenceArtifact\""), "must have CompetenceArtifact schema");
        assert!(json.contains("\"ApiError\""), "must have ApiError schema");
        assert!(json.contains("\"AcceptanceAuthority\""), "must have AcceptanceAuthority schema");
    }
}

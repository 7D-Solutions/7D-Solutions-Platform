pub mod annotations;
pub mod fields;
pub mod generate;
pub mod schemas;
pub mod submissions;
pub mod templates;
pub mod tenant;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "PDF Editor Service",
        version = "2.1.0",
        description = "PDF template management, form submissions, annotations, and document generation.",
    ),
    paths(
        templates::create_template,
        templates::list_templates,
        templates::get_template,
        templates::update_template,
        fields::create_field,
        fields::list_fields,
        fields::update_field,
        fields::reorder_fields,
        submissions::create_submission,
        submissions::list_submissions,
        submissions::get_submission,
        submissions::autosave_submission,
        submissions::submit_submission,
        generate::generate_pdf,
        annotations::render_annotations,
    ),
    components(schemas(
        crate::domain::forms::FormTemplate,
        crate::domain::forms::FormField,
        crate::domain::forms::CreateTemplateRequest,
        crate::domain::forms::UpdateTemplateRequest,
        crate::domain::forms::CreateFieldRequest,
        crate::domain::forms::UpdateFieldRequest,
        crate::domain::forms::ReorderFieldsRequest,
        crate::domain::submissions::FormSubmission,
        crate::domain::submissions::CreateSubmissionRequest,
        crate::domain::submissions::AutosaveRequest,
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<crate::domain::forms::FormTemplate>,
        platform_http_contracts::PaginatedResponse<crate::domain::submissions::FormSubmission>,
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

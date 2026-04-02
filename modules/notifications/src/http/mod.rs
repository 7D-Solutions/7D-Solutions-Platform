pub mod admin;
pub mod admin_types;
pub mod dlq;
pub mod health;
pub mod inbox;
pub mod sends;
pub mod templates;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Notifications Service",
        version = "3.0.0",
        description = "Event-driven notification delivery with scheduled dispatch and retry.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims."
    ),
    paths(
        sends::send_notification,
        sends::get_send_detail,
        sends::query_deliveries,
        templates::publish_template,
        templates::get_template,
        inbox::list_inbox,
        inbox::get_inbox_message,
        inbox::read_message,
        inbox::unread_message,
        inbox::dismiss_inbox_message,
        inbox::undismiss_inbox_message,
        dlq::list_dlq,
        dlq::get_dlq_item,
        dlq::replay_dlq_item,
        dlq::abandon_dlq_item,
    ),
    components(schemas(
        sends::SendResponse,
        sends::SendDetailResponse,
        templates::TemplateResponse,
        templates::TemplateDetailResponse,
        inbox::InboxItem,
        inbox::InboxActionResponse,
        dlq::DlqItem,
        dlq::DeliveryAttemptDetail,
        dlq::DlqDetailResponse,
        dlq::DlqActionResponse,
        platform_http_contracts::PaginatedResponse<crate::sends::models::DeliveryReceipt>,
        platform_http_contracts::PaginatedResponse<inbox::InboxItem>,
        platform_http_contracts::PaginatedResponse<dlq::DlqItem>,
        platform_http_contracts::PaginationMeta,
        crate::sends::models::SendRequest,
        crate::sends::models::DeliveryReceipt,
        crate::sends::models::DeliveryQuery,
        crate::template_store::models::CreateTemplate,
        crate::template_store::models::TemplateVersionSummary,
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

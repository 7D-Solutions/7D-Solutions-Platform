//! Utility binary that prints the Integrations OpenAPI spec as JSON to stdout.
//! No database or NATS connection required — the spec is generated at compile time.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use utoipa::OpenApi;

use integrations_rs::domain::connectors::{
    ConfigField, ConfigFieldType, ConnectorCapabilities, ConnectorConfig, RegisterConnectorRequest,
    RunTestActionRequest, TestActionResult,
};
use integrations_rs::domain::external_refs::{
    CreateExternalRefRequest, ExternalRef, UpdateExternalRefRequest,
};
use integrations_rs::domain::oauth::{ConnectionStatus, OAuthConnectionInfo};
use integrations_rs::http::qbo_invoice::{UpdateInvoiceRequest, UpdateInvoiceResponse};
use integrations_rs::http::sync::PushEntityRequest;
use platform_http_contracts::{ApiError, FieldError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Integrations Service",
        version = "2.3.0",
        description = "External system connectors, webhook routing, OAuth connection management, \
                        and reference linking.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permissions: `integrations.read` for queries, \
                        `integrations.mutate` for writes.\n\n\
                        **Webhooks:** Inbound webhooks (Stripe, GitHub, QuickBooks) are \
                        unauthenticated and gated by HMAC-SHA256 signature verification.",
    ),
    paths(
        // External Refs
        integrations_rs::http::external_refs::create_external_ref,
        integrations_rs::http::external_refs::list_by_entity,
        integrations_rs::http::external_refs::get_by_external,
        integrations_rs::http::external_refs::get_external_ref,
        integrations_rs::http::external_refs::update_external_ref,
        integrations_rs::http::external_refs::delete_external_ref,
        // Connectors
        integrations_rs::http::connectors::list_connector_types,
        integrations_rs::http::connectors::register_connector,
        integrations_rs::http::connectors::list_connectors,
        integrations_rs::http::connectors::get_connector,
        integrations_rs::http::connectors::run_connector_test,
        // OAuth
        integrations_rs::http::oauth::connect,
        integrations_rs::http::oauth::callback,
        integrations_rs::http::oauth::status,
        integrations_rs::http::oauth::disconnect,
        // Webhooks
        integrations_rs::http::webhooks::inbound_webhook,
        // QBO Invoice
        integrations_rs::http::qbo_invoice::update_invoice,
        // SyncPush
        integrations_rs::http::sync::push_entity,
    ),
    components(schemas(
        // External refs
        ExternalRef, CreateExternalRefRequest, UpdateExternalRefRequest,
        // Connectors
        ConnectorConfig, ConnectorCapabilities, ConfigField, ConfigFieldType,
        RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
        // OAuth
        OAuthConnectionInfo, ConnectionStatus,
        // QBO Invoice
        UpdateInvoiceRequest, UpdateInvoiceResponse,
        // SyncPush
        PushEntityRequest,
        // Shared envelopes
        ApiError, FieldError, PaginatedResponse<ExternalRef>,
        PaginatedResponse<ConnectorCapabilities>, PaginatedResponse<ConnectorConfig>,
        PaginationMeta,
    )),
    security(
        ("bearer" = [])
    ),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

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

fn main() {
    let spec = ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}

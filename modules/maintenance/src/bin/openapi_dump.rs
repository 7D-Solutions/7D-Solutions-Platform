//! Utility binary that prints the Maintenance OpenAPI spec as JSON to stdout.
//! No database or NATS connection required.
//!
//! Usage:  cargo run --bin openapi_dump > openapi.json

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Maintenance Service",
        version = "2.1.0",
        description = "Maintenance management: work orders, preventive plans, meters, calibration, \
                        downtime tracking, and labor management.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims.\n\
                        Permissions: MAINTENANCE_READ for queries, MAINTENANCE_MUTATE for writes."
    ),
    security(("bearer" = [])),
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

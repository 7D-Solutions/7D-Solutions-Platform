pub mod billing;
pub mod metering;
pub mod service_agreements;

use axum::{
    routing::{get, post},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;
use utoipa::OpenApi;

use crate::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "TTP Service",
        version = "3.0.0",
        description = "Tenant-to-platform billing, metering, and service agreement management.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims.\n\
                        Permissions: `ttp.read` for queries, `ttp.mutate` for writes."
    ),
    paths(
        billing::create_billing_run,
        metering::ingest_events,
        metering::get_trace,
        service_agreements::list_service_agreements,
    ),
    components(schemas(
        billing::BillingRunRequest,
        billing::BillingRunResponse,
        metering::IngestEventRequest,
        metering::EventItem,
        metering::IngestEventResponse,
        metering::IngestResultItem,
        crate::domain::metering::PriceTrace,
        crate::domain::metering::TraceLineItem,
        service_agreements::ServiceAgreementItem,
        service_agreements::ListServiceAgreementsResponse,
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

/// Build the TTP HTTP router with all endpoints.
///
/// Mutation routes (POST) require the `ttp.mutate` permission in the
/// caller's JWT.  Read routes are unenforced at this stage.
pub fn router(state: Arc<AppState>) -> Router {
    let mutations = Router::new()
        // Billing runs — write
        .route("/api/ttp/billing-runs", post(billing::create_billing_run))
        // Metering — write
        .route("/api/metering/events", post(metering::ingest_events))
        .route_layer(RequirePermissionsLayer::new(&[permissions::TTP_MUTATE]))
        .with_state(state.clone());

    let reads = Router::new()
        // Metering — read
        .route("/api/metering/trace", get(metering::get_trace))
        // Service agreements — read
        .route(
            "/api/ttp/service-agreements",
            get(service_agreements::list_service_agreements),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::TTP_READ]))
        .with_state(state.clone());

    Router::new().merge(mutations).merge(reads)
}

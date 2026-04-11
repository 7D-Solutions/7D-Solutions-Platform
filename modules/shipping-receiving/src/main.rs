use axum::{routing::get, Json, Router};
use std::sync::Arc;
use utoipa::OpenApi;

use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use shipping_receiving_rs::{
    consumers,
    db::inspection_routing_repo::InspectionRoutingRow,
    domain::{
        carrier_providers::dispatch::start_carrier_dispatch_consumer,
        inspection_routing::RouteLineRequest,
        shipments::{Direction, Shipment},
    },
    http::shipments::types::{
        AddLineRequest, CreateShipmentRequest, ReceiveLineRequest,
        ShipLineQtyRequest, ShipmentLineRow, ShipOutboundRequest, TransitionStatusRequest,
    },
    metrics, routes, AppState, QualityGateIntegration,
};
use security::{permissions, RequirePermissionsLayer};
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Shipping & Receiving Service",
        version = "3.0.0",
        description = "Inbound and outbound shipment tracking with inventory integration.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims.\n\n\
                        **Multi-tenancy:** All data scoped by tenant_id from JWT.",
    ),
    paths(
        shipping_receiving_rs::http::health::healthz,
        shipping_receiving_rs::http::health::health,
        shipping_receiving_rs::http::health::ready,
        shipping_receiving_rs::http::health::version,
        shipping_receiving_rs::http::shipments::create_shipment,
        shipping_receiving_rs::http::shipments::get_shipment,
        shipping_receiving_rs::http::shipments::list_shipments,
        shipping_receiving_rs::http::shipments::transition_status,
        shipping_receiving_rs::http::shipments::add_line,
        shipping_receiving_rs::http::shipments::receive_line,
        shipping_receiving_rs::http::shipments::ship_line_qty,
        shipping_receiving_rs::http::shipments::close_shipment,
        shipping_receiving_rs::http::shipments::ship_shipment,
        shipping_receiving_rs::http::shipments::deliver_shipment,
        shipping_receiving_rs::http::shipments::accept_line,
        shipping_receiving_rs::http::shipments::ship_outbound,
        shipping_receiving_rs::http::inspection_routing::route_line,
        shipping_receiving_rs::http::inspection_routing::list_routings,
        shipping_receiving_rs::http::refs::shipments_by_po,
        shipping_receiving_rs::http::refs::lines_by_po_line,
        shipping_receiving_rs::http::refs::shipments_by_source_ref,
    ),
    components(schemas(
        Shipment, Direction,
        CreateShipmentRequest, TransitionStatusRequest,
        AddLineRequest, ReceiveLineRequest, ShipLineQtyRequest,
        ShipmentLineRow, ShipOutboundRequest,
        RouteLineRequest, InspectionRoutingRow,
        ApiError, PaginatedResponse<Shipment>, PaginatedResponse<InspectionRoutingRow>, PaginationMeta,
    )),
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

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// SDK consumer adapter for ap.events.ap.po_approved events.
async fn on_po_approved(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let payload: consumers::po_approved::PoApprovedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    tracing::info!(event_id = %event_id, "Processing ap.events.ap.po_approved");

    consumers::po_approved::handle_po_approved(pool, event_id, &payload)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer("ap.events.ap.po_approved", on_po_approved)
        .routes(|ctx| {
            let sr_metrics = Arc::new(
                metrics::ShippingReceivingMetrics::new()
                    .expect("Failed to create metrics registry"),
            );

            let inventory = ctx.platform_client::<shipping_receiving_rs::InventoryIntegration>();
            let quality_gate = ctx.platform_client::<QualityGateIntegration>();

            // Start carrier dispatch consumer if a bus is configured
            if let Ok(bus) = ctx.bus_arc() {
                start_carrier_dispatch_consumer(bus, ctx.pool().clone());
            }

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: sr_metrics,
                inventory,
                quality_gate,
            });

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(routes::build_router())
                .merge(
                    routes::build_mutation_router().route_layer(RequirePermissionsLayer::new(&[
                        permissions::SHIPPING_RECEIVING_MUTATE,
                    ])),
                )
                .with_state(app_state)
        })
        .run()
        .await
        .expect("shipping-receiving module failed");
}

use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use shipping_receiving_rs::{
    db::inspection_routing_repo::InspectionRoutingRow,
    domain::{
        inspection_routing::RouteLineRequest,
        shipments::{Direction, Shipment},
    },
    http::shipments::types::{
        AddLineRequest, CreateShipmentRequest, ReceiveLineRequest, ShipLineQtyRequest,
        ShipmentLineRow, TransitionStatusRequest,
    },
};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Shipping & Receiving Service",
        version = "3.0.0",
        description = "Inbound and outbound shipment tracking with inventory integration.",
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
        ShipmentLineRow,
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

fn main() {
    let spec = ApiDoc::openapi();
    println!(
        "{}",
        serde_json::to_string_pretty(&spec).expect("serialize OpenAPI")
    );
}

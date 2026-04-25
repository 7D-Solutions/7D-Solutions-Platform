use axum::{
    routing::{get, patch, post},
    Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use crate::http;
use crate::AppState;

/// Ops routes — unauthenticated health/ready/version.
pub fn build_ops_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
}

/// Read routes — require shipping_receiving.read permission.
pub fn build_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/shipping-receiving/shipments",
            get(http::shipments::list_shipments),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}",
            get(http::shipments::get_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/routings",
            get(http::inspection_routing::list_routings),
        )
        .route(
            "/api/shipping-receiving/po/{po_id}/shipments",
            get(http::refs::shipments_by_po),
        )
        .route(
            "/api/shipping-receiving/po-lines/{po_line_id}/lines",
            get(http::refs::lines_by_po_line),
        )
        .route(
            "/api/shipping-receiving/source/{ref_type}/{ref_id}/shipments",
            get(http::refs::shipments_by_source_ref),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::SHIPPING_RECEIVING_READ,
        ]))
}

/// Carrier webhook routes — unauthenticated (carrier provides payload auth).
///
/// Each handler performs its own signature verification before processing.
/// ODFL and USPS return 501 as they do not support webhook push.
pub fn build_carrier_webhook_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/integrations/carriers/ups/webhook",
            post(http::carrier_webhooks::ups_webhook),
        )
        .route(
            "/api/integrations/carriers/fedex/webhook",
            post(http::carrier_webhooks::fedex_webhook),
        )
        .route(
            "/api/integrations/carriers/usps/webhook",
            post(http::carrier_webhooks::usps_webhook),
        )
        .route(
            "/api/integrations/carriers/rl/webhook",
            post(http::carrier_webhooks::rl_webhook),
        )
        .route(
            "/api/integrations/carriers/xpo/webhook",
            post(http::carrier_webhooks::xpo_webhook),
        )
        .route(
            "/api/integrations/carriers/odfl/webhook",
            post(http::carrier_webhooks::odfl_webhook),
        )
        .route(
            "/api/integrations/carriers/saia/webhook",
            post(http::carrier_webhooks::saia_webhook),
        )
}

/// Mutation routes — caller must apply RequirePermissionsLayer externally.
pub fn build_mutation_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/shipping-receiving/shipments",
            post(http::shipments::create_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/status",
            patch(http::shipments::transition_status),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/lines",
            post(http::shipments::add_line),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/lines/{line_id}/receive",
            post(http::shipments::receive_line),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/lines/{line_id}/accept",
            post(http::shipments::accept_line),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/lines/{line_id}/ship-qty",
            post(http::shipments::ship_line_qty),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/close",
            post(http::shipments::close_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/ship",
            post(http::shipments::ship_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/deliver",
            post(http::shipments::deliver_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/outbound",
            post(http::shipments::ship_outbound),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/label",
            post(http::shipments::create_label),
        )
        .route(
            "/api/shipping-receiving/returns/label",
            post(http::return_labels::create_return_label),
        )
        .route(
            "/api/shipping-receiving/shipments/{id}/lines/{line_id}/route",
            post(http::inspection_routing::route_line),
        )
        .route(
            "/api/shipping-receiving/shipments/multi-package",
            post(http::multi_package::create_multi_package_shipment),
        )
}

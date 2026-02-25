use axum::{routing::{get, patch, post}, Router};
use std::sync::Arc;

use crate::http;
use crate::AppState;

/// Read routes — accessible with any valid JWT (no extra permissions).
pub fn build_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .route(
            "/api/shipping-receiving/shipments",
            get(http::shipments::list_shipments),
        )
        .route(
            "/api/shipping-receiving/shipments/:id",
            get(http::shipments::get_shipment),
        )
        .route(
            "/api/shipping-receiving/po/:po_id/shipments",
            get(http::refs::shipments_by_po),
        )
        .route(
            "/api/shipping-receiving/po-lines/:po_line_id/lines",
            get(http::refs::lines_by_po_line),
        )
        .route(
            "/api/shipping-receiving/source/:ref_type/:ref_id/shipments",
            get(http::refs::shipments_by_source_ref),
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
            "/api/shipping-receiving/shipments/:id/status",
            patch(http::shipments::transition_status),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/lines",
            post(http::shipments::add_line),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/lines/:line_id/receive",
            post(http::shipments::receive_line),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/lines/:line_id/accept",
            post(http::shipments::accept_line),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/lines/:line_id/ship-qty",
            post(http::shipments::ship_line_qty),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/close",
            post(http::shipments::close_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/ship",
            post(http::shipments::ship_shipment),
        )
        .route(
            "/api/shipping-receiving/shipments/:id/deliver",
            post(http::shipments::deliver_shipment),
        )
}

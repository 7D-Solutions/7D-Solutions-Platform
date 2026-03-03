//! HTTP handlers for inspection routing endpoints.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::inspection_routing::{
    InspectionRoutingService, RouteLineRequest, RoutingError,
};
use crate::AppState;

use super::shipments::types::{extract_tenant, idempotency_key, ErrorBody};

fn routing_error_response(err: RoutingError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        RoutingError::ShipmentNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Shipment not found")),
        ),
        RoutingError::LineNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Shipment line not found")),
        ),
        RoutingError::NotInbound => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(
                "validation_error",
                "Routing is only valid for inbound shipments",
            )),
        ),
        RoutingError::NotReceiving { ref current } => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(
                "validation_error",
                &format!(
                    "Shipment must be in receiving status to route (current: {current})"
                ),
            )),
        ),
        RoutingError::AlreadyRouted { decision, .. } => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "already_routed",
                &format!("Line is already routed as '{decision}'"),
            )),
        ),
        RoutingError::InvalidDecision(ref msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", msg)),
        ),
        RoutingError::Database(ref e) => {
            tracing::error!("database error in routing: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Internal server error")),
            )
        }
    }
}

/// POST /api/shipping-receiving/shipments/{id}/lines/{line_id}/route
pub async fn route_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
    Json(mut req): Json<RouteLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // Use header idempotency key if body doesn't provide one
    if req.idempotency_key.is_none() {
        req.idempotency_key = idempotency_key(&headers);
    }

    let routed_by = claims.as_ref().map(|Extension(c)| c.user_id);

    match InspectionRoutingService::route_line(
        &state.pool,
        shipment_id,
        line_id,
        tenant_id,
        routed_by,
        &req,
    )
    .await
    {
        Ok(routing) => (StatusCode::CREATED, Json(json!(routing))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// GET /api/shipping-receiving/shipments/{id}/routings
pub async fn list_routings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(shipment_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match InspectionRoutingService::list_for_shipment(&state.pool, shipment_id, tenant_id).await {
        Ok(routings) => (StatusCode::OK, Json(json!(routings))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

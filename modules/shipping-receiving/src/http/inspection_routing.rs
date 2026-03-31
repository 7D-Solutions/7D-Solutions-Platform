//! HTTP handlers for inspection routing endpoints.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::inspection_routing::{InspectionRoutingService, RouteLineRequest};
use crate::AppState;

use super::shipments::types::{extract_tenant, idempotency_key, with_request_id};

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{shipment_id}/lines/{line_id}/route",
    tag = "Inspection Routing",
    params(
        ("shipment_id" = Uuid, Path, description = "Shipment ID"),
        ("line_id" = Uuid, Path, description = "Shipment line ID"),
    ),
    request_body = RouteLineRequest,
    responses(
        (status = 201, description = "Line routed", body = crate::db::inspection_routing_repo::InspectionRoutingRow),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Already routed", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn route_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path((shipment_id, line_id)): Path<(Uuid, Uuid)>,
    Json(mut req): Json<RouteLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
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
        Ok(routing) => (StatusCode::CREATED, Json(serde_json::json!(routing))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/shipping-receiving/shipments/{id}/routings",
    tag = "Inspection Routing",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    responses(
        (status = 200, description = "Routings for shipment", body = Vec<crate::db::inspection_routing_repo::InspectionRoutingRow>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_routings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(shipment_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match InspectionRoutingService::list_for_shipment(&state.pool, shipment_id, tenant_id).await {
        Ok(routings) => (StatusCode::OK, Json(serde_json::json!(routings))).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

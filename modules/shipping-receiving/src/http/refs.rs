//! Query endpoints for AP/AR ref linkage.
//!
//! - GET /api/shipping-receiving/po/:po_id/shipments
//! - GET /api/shipping-receiving/po-lines/:po_line_id/lines
//! - GET /api/shipping-receiving/source/:ref_type/:ref_id/shipments

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repository::ShipmentRepository;
use crate::domain::shipments::ShipmentError;
use platform_sdk::extract_tenant;
use crate::http::shipments::types::with_request_id;
use crate::AppState;

#[utoipa::path(
    get,
    path = "/api/shipping-receiving/po/{po_id}/shipments",
    tag = "Refs",
    params(("po_id" = Uuid, Path, description = "Purchase order ID")),
    responses(
        (status = 200, description = "Shipments for PO", body = Vec<crate::domain::shipments::Shipment>),
    ),
    security(("bearer" = [])),
)]
pub async fn shipments_by_po(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(po_id): Path<Uuid>,
) -> Response {
    let tenant_id: Uuid = match extract_tenant(&claims) {
        Ok(id) => id.parse().expect("tenant_id is a valid UUID"),
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ShipmentRepository::find_shipments_by_po(&state.pool, tenant_id, po_id).await {
        Ok(shipments) => Json(serde_json::json!(shipments)).into_response(),
        Err(e) => {
            with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
                .into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/shipping-receiving/po-lines/{po_line_id}/lines",
    tag = "Refs",
    params(("po_line_id" = Uuid, Path, description = "PO line ID")),
    responses(
        (status = 200, description = "Lines for PO line", body = Vec<crate::http::shipments::ShipmentLineRow>),
    ),
    security(("bearer" = [])),
)]
pub async fn lines_by_po_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(po_line_id): Path<Uuid>,
) -> Response {
    let tenant_id: Uuid = match extract_tenant(&claims) {
        Ok(id) => id.parse().expect("tenant_id is a valid UUID"),
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ShipmentRepository::find_lines_by_po_line(&state.pool, tenant_id, po_line_id).await {
        Ok(lines) => Json(serde_json::json!(lines)).into_response(),
        Err(e) => {
            with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
                .into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/shipping-receiving/source/{ref_type}/{ref_id}/shipments",
    tag = "Refs",
    params(
        ("ref_type" = String, Path, description = "Source reference type"),
        ("ref_id" = Uuid, Path, description = "Source reference ID"),
    ),
    responses(
        (status = 200, description = "Shipments by source ref", body = Vec<crate::domain::shipments::Shipment>),
    ),
    security(("bearer" = [])),
)]
pub async fn shipments_by_source_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path((ref_type, ref_id)): Path<(String, Uuid)>,
) -> Response {
    let tenant_id: Uuid = match extract_tenant(&claims) {
        Ok(id) => id.parse().expect("tenant_id is a valid UUID"),
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ShipmentRepository::find_by_source_ref(&state.pool, tenant_id, &ref_type, ref_id).await {
        Ok(shipments) => Json(serde_json::json!(shipments)).into_response(),
        Err(e) => {
            with_request_id(ApiError::from(ShipmentError::Database(e)), &tracing_ctx)
                .into_response()
        }
    }
}

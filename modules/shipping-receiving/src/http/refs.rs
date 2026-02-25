//! Query endpoints for AP/AR ref linkage.
//!
//! - GET /api/shipping-receiving/po/:po_id/shipments
//! - GET /api/shipping-receiving/po-lines/:po_line_id/lines
//! - GET /api/shipping-receiving/source/:ref_type/:ref_id/shipments

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::repository::ShipmentRepository;
use crate::domain::shipments::ShipmentError;
use crate::http::shipments::types::{error_response, extract_tenant};
use crate::AppState;

/// GET /api/shipping-receiving/po/:po_id/shipments
///
/// Returns all shipments with at least one inbound line referencing the given PO.
pub async fn shipments_by_po(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(po_id): Path<Uuid>,
) -> Response {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ShipmentRepository::find_shipments_by_po(&state.pool, tenant_id, po_id).await {
        Ok(shipments) => (StatusCode::OK, Json(json!(shipments))).into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

/// GET /api/shipping-receiving/po-lines/:po_line_id/lines
///
/// Returns all shipment lines linked to the given PO line.
pub async fn lines_by_po_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(po_line_id): Path<Uuid>,
) -> Response {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ShipmentRepository::find_lines_by_po_line(&state.pool, tenant_id, po_line_id).await {
        Ok(lines) => (StatusCode::OK, Json(json!(lines))).into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

/// GET /api/shipping-receiving/source/:ref_type/:ref_id/shipments
///
/// Returns all shipments with at least one line matching the given source ref.
pub async fn shipments_by_source_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((ref_type, ref_id)): Path<(String, Uuid)>,
) -> Response {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ShipmentRepository::find_by_source_ref(&state.pool, tenant_id, &ref_type, ref_id).await {
        Ok(shipments) => (StatusCode::OK, Json(json!(shipments))).into_response(),
        Err(e) => error_response(ShipmentError::Database(e)).into_response(),
    }
}

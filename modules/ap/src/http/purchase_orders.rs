//! HTTP handlers for purchase order CRUD and lifecycle.
//!
//! POST /api/ap/pos                    — create a draft PO with line items
//! GET  /api/ap/pos                    — list POs for tenant
//! GET  /api/ap/pos/:po_id             — get a single PO with its lines
//! PUT  /api/ap/pos/:po_id/lines       — replace all lines on a draft PO (idempotent)
//! POST /api/ap/pos/:po_id/approve     — approve a draft PO (idempotent)

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::po::{
    approve, service, ApprovePoRequest, CreatePoRequest, PoError, PurchaseOrder,
    PurchaseOrderWithLines, UpdatePoLinesRequest,
};
use crate::http::vendors::ErrorBody;
use crate::AppState;

// ============================================================================
// Shared helpers
// ============================================================================

fn tenant_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_tenant", "X-Tenant-Id header is required")),
            )
        })
}

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn po_error_response(e: PoError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        PoError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("po_not_found", &format!("PO {} not found", id))),
        ),
        PoError::VendorNotFound(id) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "vendor_not_found",
                &format!("Vendor {} not found or inactive", id),
            )),
        ),
        PoError::NotDraft(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "po_not_draft",
                &format!("PO cannot be edited; current status: {}", status),
            )),
        ),
        PoError::InvalidTransition { from, to } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "invalid_transition",
                &format!("Cannot transition PO from '{}' to '{}'", from, to),
            )),
        ),
        PoError::EmptyLines => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("empty_lines", "PO must have at least one line")),
        ),
        PoError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        PoError::Database(e) => {
            tracing::error!("AP purchase_orders DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListPosQuery {
    /// Filter to a specific vendor
    pub vendor_id: Option<Uuid>,
    /// Filter by status (draft, approved, closed, cancelled)
    pub status: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/pos — create a draft PO with line items
pub async fn create_po(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreatePoRequest>,
) -> Result<(StatusCode, Json<PurchaseOrderWithLines>), (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let po = service::create_po(&state.pool, &tenant_id, &req, correlation_id)
        .await
        .map_err(po_error_response)?;

    Ok((StatusCode::CREATED, Json(po)))
}

/// GET /api/ap/pos/:po_id — get a single PO with its lines
pub async fn get_po(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(po_id): Path<Uuid>,
) -> Result<Json<PurchaseOrderWithLines>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let po = service::get_po(&state.pool, &tenant_id, po_id)
        .await
        .map_err(po_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new("po_not_found", &format!("PO {} not found", po_id))),
            )
        })?;

    Ok(Json(po))
}

/// GET /api/ap/pos — list POs for tenant (optionally filtered by vendor_id or status)
pub async fn list_pos(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListPosQuery>,
) -> Result<Json<Vec<PurchaseOrder>>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let pos = service::list_pos(
        &state.pool,
        &tenant_id,
        query.vendor_id,
        query.status.as_deref(),
    )
    .await
    .map_err(po_error_response)?;

    Ok(Json(pos))
}

/// PUT /api/ap/pos/:po_id/lines — replace all lines on a draft PO (idempotent)
pub async fn update_po_lines(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(po_id): Path<Uuid>,
    Json(req): Json<UpdatePoLinesRequest>,
) -> Result<Json<PurchaseOrderWithLines>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let po = service::update_po_lines(&state.pool, &tenant_id, po_id, &req)
        .await
        .map_err(po_error_response)?;

    Ok(Json(po))
}

/// POST /api/ap/pos/:po_id/approve — approve a draft PO (idempotent)
///
/// Transitions the PO from draft → approved and emits ap.po_approved.
/// If the PO is already approved, returns 200 with the current state (no re-emit).
/// Returns 422 if the PO is in a terminal state that cannot be approved.
pub async fn approve_po(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(po_id): Path<Uuid>,
    Json(req): Json<ApprovePoRequest>,
) -> Result<Json<PurchaseOrder>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let po = approve::approve_po(&state.pool, &tenant_id, po_id, &req, correlation_id)
        .await
        .map_err(po_error_response)?;

    Ok(Json(po))
}

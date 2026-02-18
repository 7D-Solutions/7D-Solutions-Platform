//! HTTP handlers for vendor bill CRUD.
//!
//! POST /api/ap/bills        — create a vendor bill
//! GET  /api/ap/bills        — list bills for tenant (filter by vendor, voided)
//! GET  /api/ap/bills/:id    — get a single bill with its line items

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::bills::{
    service, BillError, CreateBillRequest, VendorBill, VendorBillWithLines,
};
use crate::http::vendors::ErrorBody;
use crate::AppState;

// ============================================================================
// Shared helpers (local to bills; mirrors vendors.rs helpers)
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

fn bill_error_response(e: BillError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        BillError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("bill_not_found", &format!("Bill {} not found", id))),
        ),
        BillError::VendorNotFound(id) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "vendor_not_found",
                &format!("Vendor {} not found or inactive", id),
            )),
        ),
        BillError::DuplicateInvoice(ref_) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_invoice",
                &format!("Invoice '{}' already exists for this vendor", ref_),
            )),
        ),
        BillError::InvalidTransition { from, to } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "invalid_transition",
                &format!("Cannot transition bill from '{}' to '{}'", from, to),
            )),
        ),
        BillError::EmptyLines => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("empty_lines", "Bill must have at least one line")),
        ),
        BillError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        BillError::Database(e) => {
            tracing::error!("AP bills DB error: {}", e);
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
pub struct ListBillsQuery {
    /// Filter to a specific vendor
    pub vendor_id: Option<Uuid>,
    /// Include voided bills (default: false)
    #[serde(default)]
    pub include_voided: bool,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/bills — create a vendor bill
pub async fn create_bill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateBillRequest>,
) -> Result<(StatusCode, Json<VendorBillWithLines>), (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;
    let correlation_id = correlation_from_headers(&headers);

    let bill = service::create_bill(&state.pool, &tenant_id, &req, correlation_id)
        .await
        .map_err(bill_error_response)?;

    Ok((StatusCode::CREATED, Json(bill)))
}

/// GET /api/ap/bills/:bill_id — get a single bill with its line items
pub async fn get_bill(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
) -> Result<Json<VendorBillWithLines>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let bill = service::get_bill(&state.pool, &tenant_id, bill_id)
        .await
        .map_err(bill_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "bill_not_found",
                    &format!("Bill {} not found", bill_id),
                )),
            )
        })?;

    Ok(Json(bill))
}

/// GET /api/ap/bills — list bills for tenant
pub async fn list_bills(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListBillsQuery>,
) -> Result<Json<Vec<VendorBill>>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let bills = service::list_bills(
        &state.pool,
        &tenant_id,
        query.vendor_id,
        query.include_voided,
    )
    .await
    .map_err(bill_error_response)?;

    Ok(Json(bills))
}

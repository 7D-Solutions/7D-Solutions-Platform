//! HTTP handlers for AP allocation endpoints.
//!
//! POST /api/ap/bills/:bill_id/allocations — apply a payment allocation to a bill
//! GET  /api/ap/bills/:bill_id/allocations — list all allocations for a bill
//! GET  /api/ap/bills/:bill_id/balance     — get remaining open balance for a bill

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::allocations::{service, AllocationError, CreateAllocationRequest};
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

fn allocation_error_response(e: AllocationError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        AllocationError::BillNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("bill_not_found", &format!("Bill {} not found", id))),
        ),
        AllocationError::InvalidBillStatus(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "invalid_bill_status",
                &format!(
                    "Bill status '{}' does not accept allocations; \
                     bill must be 'approved' or 'partially_paid'",
                    status
                ),
            )),
        ),
        AllocationError::OverAllocation { available, requested } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "over_allocation",
                &format!(
                    "Allocation of {} would exceed open balance of {}",
                    requested, available
                ),
            )),
        ),
        AllocationError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        AllocationError::Database(e) => {
            tracing::error!(error = %e, "Database error in allocation handler");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "An internal error occurred")),
            )
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/bills/:bill_id/allocations
///
/// Apply a payment allocation to an approved or partially-paid bill.
/// Idempotent: duplicate allocation_id returns the existing record (200 OK).
pub async fn create_allocation(
    State(state): State<Arc<AppState>>,
    Path(bill_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CreateAllocationRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let record = service::apply_allocation(&state.pool, &tenant_id, bill_id, &req)
        .await
        .map_err(allocation_error_response)?;

    Ok(Json(json!({
        "allocation_id": record.allocation_id,
        "bill_id": record.bill_id,
        "amount_minor": record.amount_minor,
        "currency": record.currency,
        "allocation_type": record.allocation_type,
        "payment_run_id": record.payment_run_id,
        "created_at": record.created_at,
    })))
}

/// GET /api/ap/bills/:bill_id/allocations
///
/// List all allocations for a bill in insertion order.
pub async fn list_allocations(
    State(state): State<Arc<AppState>>,
    Path(bill_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let records = service::get_allocations(&state.pool, &tenant_id, bill_id)
        .await
        .map_err(allocation_error_response)?;

    Ok(Json(json!({ "allocations": records })))
}

/// GET /api/ap/bills/:bill_id/balance
///
/// Return remaining open balance for a bill.
pub async fn get_balance(
    State(state): State<Arc<AppState>>,
    Path(bill_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let summary = service::get_bill_balance(&state.pool, &tenant_id, bill_id)
        .await
        .map_err(allocation_error_response)?;

    match summary {
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("bill_not_found", &format!("Bill {} not found", bill_id))),
        )),
        Some(s) => Ok(Json(json!({
            "bill_id": s.bill_id,
            "total_minor": s.total_minor,
            "allocated_minor": s.allocated_minor,
            "open_balance_minor": s.open_balance_minor,
            "status": s.status,
        }))),
    }
}

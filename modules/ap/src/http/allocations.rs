//! HTTP handlers for AP allocation endpoints.
//!
//! POST /api/ap/bills/:bill_id/allocations — apply a payment allocation to a bill
//! GET  /api/ap/bills/:bill_id/allocations — list all allocations for a bill
//! GET  /api/ap/bills/:bill_id/balance     — get remaining open balance for a bill

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::allocations::{service, AllocationRecord, CreateAllocationRequest};
use platform_sdk::extract_tenant;
use crate::http::tenant::with_request_id;
use crate::AppState;

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/bills/:bill_id/allocations
///
/// Apply a payment allocation to an approved or partially-paid bill.
/// Idempotent: duplicate allocation_id returns the existing record (200 OK).
#[utoipa::path(post, path = "/api/ap/bills/{bill_id}/allocations", tag = "Allocations",
    params(("bill_id" = Uuid, Path)), request_body = CreateAllocationRequest,
    responses((status = 200, description = "Allocation applied", body = AllocationRecord)), security(("bearer" = [])))]
pub async fn create_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<CreateAllocationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::apply_allocation(&state.pool, &tenant_id, bill_id, &req).await {
        Ok(record) => Json(record).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/bills/:bill_id/allocations
///
/// List all allocations for a bill in insertion order.
#[utoipa::path(get, path = "/api/ap/bills/{bill_id}/allocations", tag = "Allocations",
    params(("bill_id" = Uuid, Path)),
    responses((status = 200, description = "Allocation list", body = PaginatedResponse<crate::domain::allocations::AllocationRecord>)),
    security(("bearer" = [])))]
pub async fn list_allocations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(bill_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::get_allocations(&state.pool, &tenant_id, bill_id).await {
        Ok(records) => {
            let total = records.len() as i64;
            let resp = PaginatedResponse::new(records, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/bills/:bill_id/balance
///
/// Return remaining open balance for a bill.
#[utoipa::path(get, path = "/api/ap/bills/{bill_id}/balance", tag = "Allocations",
    params(("bill_id" = Uuid, Path)), responses((status = 200, description = "Bill balance", body = crate::domain::allocations::BillBalanceSummary)),
    security(("bearer" = [])))]
pub async fn get_balance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(bill_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::get_bill_balance(&state.pool, &tenant_id, bill_id).await {
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Bill {} not found", bill_id)),
            &tracing_ctx,
        )
        .into_response(),
        Ok(Some(s)) => Json(s).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

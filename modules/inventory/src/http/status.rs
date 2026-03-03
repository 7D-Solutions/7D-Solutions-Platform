//! Status transfer HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/status-transfers
//!     — move quantity between status buckets (available/quarantine/damaged)
//!
//! Idempotency:
//!   Callers MUST supply `idempotency_key` in the request body.
//!   Duplicate keys with the same body return 200 OK with the stored result.
//!   Duplicate keys with a different body return 409 Conflict.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;

use super::tenant::extract_tenant;
use crate::{
    domain::{
        guards::GuardError,
        status::transfer_service::{
            process_status_transfer, StatusTransferError as TransferError, StatusTransferRequest,
        },
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn transfer_error_response(err: TransferError) -> impl IntoResponse {
    match err {
        TransferError::Guard(GuardError::ItemNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "item_not_found",
                "message": "Item not found or does not belong to this tenant"
            })),
        ),
        TransferError::Guard(GuardError::ItemInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "item_inactive",
                "message": "Item is inactive and cannot have stock transferred"
            })),
        ),
        TransferError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        TransferError::Guard(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": "Guard check failed" })),
        ),
        TransferError::SameStatus => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "same_status",
                "message": "from_status and to_status must differ"
            })),
        ),
        TransferError::InsufficientStock {
            status,
            available,
            requested,
        } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "insufficient_stock",
                "message": format!(
                    "Insufficient stock in {} bucket: have {}, need {}",
                    status, available, requested
                )
            })),
        ),
        TransferError::BucketNotFound(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "bucket_not_found",
                "message": format!("No {} bucket found for this item/warehouse", status)
            })),
        ),
        TransferError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "Idempotency key already used with a different request body"
            })),
        ),
        TransferError::Serialization(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "serialization_error", "message": e.to_string() })),
        ),
        TransferError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/status-transfers
///
/// Moves quantity between status buckets atomically.
/// Returns 201 on new transfer, 200 on idempotent replay.
pub async fn post_status_transfer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<StatusTransferRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match process_status_transfer(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(err) => transfer_error_response(err).into_response(),
    }
}

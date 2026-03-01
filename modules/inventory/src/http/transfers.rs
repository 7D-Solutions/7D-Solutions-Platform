//! Stock transfer HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/transfers — move stock between warehouses (paired ledger entries)
//!
//! Tenant identity derived from JWT `VerifiedClaims`.
//!
//! Idempotency:
//!   Callers MUST supply `idempotency_key` in the request body.
//!   Duplicate keys with the same body return 200 OK with the stored result.
//!   Duplicate keys with a different body return 409 Conflict.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;

use super::tenant::extract_tenant;
use crate::{
    domain::{
        guards::GuardError,
        transfer_service::{process_transfer, TransferError, TransferRequest},
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
        )
            .into_response(),
        TransferError::Guard(GuardError::ItemInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "item_inactive",
                "message": "Item is inactive and cannot be transferred"
            })),
        )
            .into_response(),
        TransferError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        TransferError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error in transfer");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
        TransferError::Guard(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "guard_error", "message": e.to_string() })),
        )
            .into_response(),
        TransferError::SameWarehouse => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "same_warehouse",
                "message": "Source and destination warehouse must be different"
            })),
        )
            .into_response(),
        TransferError::InsufficientQuantity {
            requested,
            available,
        } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "insufficient_quantity",
                "message": format!(
                    "Insufficient stock: requested {}, available {}",
                    requested, available
                )
            })),
        )
            .into_response(),
        TransferError::Fifo(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "fifo_error", "message": e.to_string() })),
        )
            .into_response(),
        TransferError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "The idempotency key was previously used with a different request body"
            })),
        )
            .into_response(),
        TransferError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in transfer");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
                .into_response()
        }
        TransferError::Database(e) => {
            tracing::error!(error = %e, "database error in transfer");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/transfers
///
/// Moves stock between warehouses as paired ledger entries (transfer_out + transfer_in)
/// in a single atomic transaction. FIFO is consumed on the source side; a new cost
/// layer is created at the destination at weighted average cost.
///
/// Returns 201 Created on new transfer; 200 OK on idempotency replay.
pub async fn post_transfer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<TransferRequest>,
) -> impl IntoResponse {
    let tracing_ctx = tracing_ctx.map(|Extension(c)| c).unwrap_or_default();
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match process_transfer(&state.pool, &req, Some(&tracing_ctx)).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(result)).into_response()
        }
        Err(err) => transfer_error_response(err).into_response(),
    }
}

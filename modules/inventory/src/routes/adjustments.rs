//! Stock adjustment HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/adjustments — create a compensating ledger entry
//!
//! Idempotency:
//!   Callers MUST supply `idempotency_key` in the request body.
//!   Duplicate keys with the same body return 200 OK with the stored result.
//!   Duplicate keys with a different body return 409 Conflict.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::sync::Arc;

use crate::{
    domain::{
        adjust_service::{process_adjustment, AdjustError, AdjustRequest},
        guards::GuardError,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn adjust_error_response(err: AdjustError) -> impl IntoResponse {
    match err {
        AdjustError::Guard(GuardError::ItemNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "item_not_found",
                "message": "Item not found or does not belong to this tenant"
            })),
        )
            .into_response(),
        AdjustError::Guard(GuardError::ItemInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "item_inactive",
                "message": "Item is inactive and cannot be adjusted"
            })),
        )
            .into_response(),
        AdjustError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        AdjustError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error in adjustment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
        AdjustError::Guard(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "guard_error", "message": e.to_string() })),
        )
            .into_response(),
        AdjustError::NegativeOnHand { available, would_be } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "negative_on_hand",
                "message": format!(
                    "Adjustment would drive on-hand negative: have {}, would become {}",
                    available, would_be
                )
            })),
        )
            .into_response(),
        AdjustError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "The idempotency key was previously used with a different request body"
            })),
        )
            .into_response(),
        AdjustError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in adjustment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
                .into_response()
        }
        AdjustError::Database(e) => {
            tracing::error!(error = %e, "database error in adjustment");
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

/// POST /api/inventory/adjustments
///
/// Creates a compensating ledger entry to correct physical reality.
/// Returns 201 Created on new adjustment; 200 OK on idempotency replay.
pub async fn post_adjustment(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdjustRequest>,
) -> impl IntoResponse {
    match process_adjustment(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(result)).into_response()
        }
        Err(err) => adjust_error_response(err).into_response(),
    }
}

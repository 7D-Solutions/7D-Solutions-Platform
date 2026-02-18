//! Stock receipt HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/receipts — atomic receipt: ledger row + FIFO layer + outbox event
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
        guards::GuardError,
        receipt_service::{process_receipt, ReceiptError, ReceiptRequest},
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn receipt_error_response(err: ReceiptError) -> impl IntoResponse {
    match err {
        ReceiptError::Guard(GuardError::ItemNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "item_not_found",
                "message": "Item not found or does not belong to this tenant"
            })),
        ),
        ReceiptError::Guard(GuardError::ItemInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "item_inactive",
                "message": "Item is inactive and cannot receive stock"
            })),
        ),
        ReceiptError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ReceiptError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
        ReceiptError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "Idempotency key already used with a different request body"
            })),
        ),
        ReceiptError::Serialization(e) => {
            tracing::error!(error = %e, "receipt serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        ReceiptError::Database(e) => {
            tracing::error!(error = %e, "receipt database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/receipts
///
/// Creates a stock receipt atomically: ledger row + FIFO layer + outbox event,
/// all in a single database transaction.
///
/// Responses:
///   201 Created  — receipt created, rows committed
///   200 OK       — idempotency replay (same key + same body, stored result returned)
///   409 Conflict — same idempotency key with a different request body
///   404 Not Found — item not found or wrong tenant
///   422 Unprocessable Entity — validation failure (inactive item, zero qty, zero cost)
///   500 Internal Server Error — unexpected error
pub async fn post_receipt(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReceiptRequest>,
) -> impl IntoResponse {
    match process_receipt(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => receipt_error_response(e).into_response(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    /// DB integration tests live in the integration test suite (cargo test -p inventory).
    /// Unit tests for request/response parsing belong here.

    #[test]
    fn placeholder_receipts_module_compiles() {
        // Ensures this module compiles cleanly.
    }
}

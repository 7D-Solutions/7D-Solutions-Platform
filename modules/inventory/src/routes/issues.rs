//! Stock issue HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/issues — atomic issue: ledger row + FIFO layer consumptions
//!                                + on-hand projection + outbox event (inventory.item_issued)
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
        issue_service::{process_issue, IssueError, IssueRequest},
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn issue_error_response(err: IssueError) -> impl IntoResponse {
    match err {
        IssueError::Guard(GuardError::ItemNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "item_not_found",
                "message": "Item not found or does not belong to this tenant"
            })),
        ),
        IssueError::Guard(GuardError::ItemInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "item_inactive",
                "message": "Item is inactive and cannot be issued"
            })),
        ),
        IssueError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        IssueError::Guard(GuardError::NoBaseUom) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_base_uom",
                "message": "Item has no base_uom configured; cannot convert input UoM"
            })),
        ),
        IssueError::Guard(GuardError::UomConversion(e)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "uom_conversion_error", "message": e.to_string() })),
        ),
        IssueError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error in issue");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
        IssueError::InsufficientQuantity { requested, available } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "insufficient_quantity",
                "message": format!(
                    "Insufficient stock: requested {requested}, available {available}"
                )
            })),
        ),
        IssueError::Fifo(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "fifo_error", "message": e.to_string() })),
        ),
        IssueError::NoLayersAvailable => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_stock",
                "message": "No stock layers available for this item/warehouse"
            })),
        ),
        IssueError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "Idempotency key already used with a different request body"
            })),
        ),
        IssueError::Serialization(e) => {
            tracing::error!(error = %e, "issue serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        IssueError::Database(e) => {
            tracing::error!(error = %e, "issue database error");
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

/// POST /api/inventory/issues
///
/// Issues stock atomically: ledger row (negative qty) + layer_consumptions +
/// updated FIFO layers + on-hand projection + outbox event, all in one transaction.
///
/// Responses:
///   201 Created  — issue created, rows committed
///   200 OK       — idempotency replay (same key + same body)
///   409 Conflict — same idempotency key with a different request body
///   404 Not Found — item not found or wrong tenant
///   422 Unprocessable Entity — validation failure or insufficient stock
///   500 Internal Server Error — unexpected error
pub async fn post_issue(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IssueRequest>,
) -> impl IntoResponse {
    match process_issue(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => issue_error_response(e).into_response(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_issues_module_compiles() {
        // DB integration tests live in tests/issue_integration.rs
    }
}

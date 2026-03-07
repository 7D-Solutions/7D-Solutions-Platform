//! Reservation HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/reservations/reserve          — create a stock hold
//!   POST /api/inventory/reservations/release          — compensating release referencing a reserve
//!   POST /api/inventory/reservations/{id}/fulfill     — fulfill reservation (physical stock deduction)
//!
//! Tenant identity derived from JWT `VerifiedClaims`.
//!
//! Idempotency:
//!   Callers MUST supply `idempotency_key` in the request body.
//!   Duplicate keys with the same body return 200 OK with the stored result.
//!   Duplicate keys with a different body return 409 Conflict.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::{
        fulfill_service::{process_fulfill, FulfillError, FulfillRequest},
        guards::GuardError,
        reservation_service::{
            process_release, process_reserve, ReleaseRequest, ReservationError, ReserveRequest,
        },
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn reservation_error_response(err: ReservationError) -> impl IntoResponse {
    match err {
        ReservationError::Guard(GuardError::ItemNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "item_not_found",
                "message": "Item not found or does not belong to this tenant"
            })),
        ),
        ReservationError::Guard(GuardError::ItemInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "item_inactive",
                "message": "Item is inactive and cannot be reserved"
            })),
        ),
        ReservationError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ReservationError::Guard(GuardError::NoBaseUom) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_base_uom",
                "message": "Item has no base_uom configured; cannot convert input UoM"
            })),
        ),
        ReservationError::Guard(GuardError::UomConversion(e)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "uom_conversion_error", "message": e.to_string() })),
        ),
        ReservationError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
        ReservationError::ReservationNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "reservation_not_found",
                "message": "Reservation not found or does not belong to this tenant"
            })),
        ),
        ReservationError::AlreadyReleased => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_released",
                "message": "Reservation already has a compensating release or fulfillment entry"
            })),
        ),
        ReservationError::InsufficientAvailable { requested, available } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "insufficient_available",
                "message": format!("Insufficient available stock: requested {}, available {}", requested, available)
            })),
        ),
        ReservationError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "Idempotency key already used with a different request body"
            })),
        ),
        ReservationError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        ReservationError::Database(e) => {
            tracing::error!(error = %e, "reservation database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/reservations/reserve
///
/// Creates a stock hold atomically: reservation row + on-hand projection update
/// + outbox event, all in a single database transaction.
///
/// Responses:
///   201 Created   — reservation created
///   200 OK        — idempotency replay
///   409 Conflict  — idempotency key conflict, or item not found/inactive
///   422           — validation failure
///   500           — internal error
pub async fn post_reserve(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<ReserveRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match process_reserve(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => reservation_error_response(e).into_response(),
    }
}

/// POST /api/inventory/reservations/release
///
/// Creates a compensating release row referencing the original reserve, decrements
/// the on-hand projection, and writes an outbox event, all in one transaction.
///
/// Responses:
///   200 OK        — released (or idempotency replay)
///   404 Not Found — reservation_id not found or wrong tenant
///   409 Conflict  — already released / idempotency conflict
///   422           — validation failure
///   500           — internal error
pub async fn post_release(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<ReleaseRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match process_release(&state.pool, &req).await {
        Ok((result, _is_replay)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => reservation_error_response(e).into_response(),
    }
}

// ============================================================================
// Fulfill error mapping
// ============================================================================

fn fulfill_error_response(err: FulfillError) -> impl IntoResponse {
    match err {
        FulfillError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        FulfillError::Guard(e) => {
            tracing::error!(error = %e, "fulfill guard error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": e.to_string() })),
            )
        }
        FulfillError::ReservationNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "reservation_not_found",
                "message": "Reservation not found or does not belong to this tenant"
            })),
        ),
        FulfillError::AlreadySettled => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_settled",
                "message": "Reservation already fulfilled or released"
            })),
        ),
        FulfillError::QuantityExceedsReserved(requested, reserved) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "quantity_exceeds_reserved",
                "message": format!("Fulfill quantity {} exceeds reserved quantity {}", requested, reserved)
            })),
        ),
        FulfillError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "Idempotency key already used with a different request body"
            })),
        ),
        FulfillError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in fulfill");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        FulfillError::Database(e) => {
            tracing::error!(error = %e, "fulfill database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/inventory/reservations/{reservation_id}/fulfill
///
/// Converts an active reservation into a physical stock deduction.
/// Creates a compensating 'fulfilled' row, decrements quantity_reserved
/// AND quantity_on_hand, and writes an outbox event, all in one transaction.
///
/// Responses:
///   200 OK        — fulfilled (or idempotency replay)
///   404 Not Found — reservation_id not found or wrong tenant
///   409 Conflict  — already settled / idempotency conflict
///   422           — validation failure (quantity > reserved)
///   500           — internal error
pub async fn post_fulfill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(reservation_id): Path<Uuid>,
    Json(mut req): Json<FulfillRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    // Inject path param into request body for consistency.
    req.reservation_id = reservation_id;
    match process_fulfill(&state.pool, &req).await {
        Ok((result, _is_replay)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => fulfill_error_response(e).into_response(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    /// DB integration tests live in modules/inventory/tests/reservation_integration.rs
    /// and e2e-tests/tests/inventory_reservation_e2e.rs.

    #[test]
    fn placeholder_reservations_module_compiles() {
        // Ensures this module compiles cleanly.
    }
}

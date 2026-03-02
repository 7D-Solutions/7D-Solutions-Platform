//! Lot and serial traceability HTTP handlers.
//!
//! Endpoints:
//!   GET /api/inventory/items/{item_id}/lots/{lot_code}/trace
//!   GET /api/inventory/items/{item_id}/serials/{serial_code}/trace
//!
//! Both return the ledger movement history for the given code, oldest-first.
//! Tenant derived from JWT VerifiedClaims.

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

use crate::{
    domain::lots_serials::queries::{trace_lot, trace_serial},
    AppState,
};

// ============================================================================
// Handlers
// ============================================================================

/// GET /api/inventory/items/{item_id}/lots/{lot_code}/trace
///
/// Returns all ledger movements associated with the given lot code for the item.
/// Movements include the original receipt and any subsequent issues or transfers
/// from the lot's FIFO layers, ordered chronologically by ledger id.
///
/// Responds with 200 + `{ "lot_code": "...", "movements": [...] }`.
/// Empty `movements` array when the lot has no entries (e.g. lot not yet found).
pub async fn trace_lot_handler(
    State(state): State<Arc<AppState>>,
    Path((item_id, lot_code)): Path<(Uuid, String)>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    match trace_lot(&state.pool, &tenant_id, item_id, &lot_code).await {
        Ok(movements) => (
            StatusCode::OK,
            Json(json!({
                "lot_code": lot_code,
                "item_id": item_id,
                "movements": movements
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                error = %e,
                item_id = %item_id,
                lot_code = %lot_code,
                "database error tracing lot"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// GET /api/inventory/items/{item_id}/serials/{serial_code}/trace
///
/// Returns all ledger movements associated with the given serial code for the item.
/// Movements include the original receipt and any subsequent issue entries for
/// the FIFO layer this serial occupied, ordered chronologically by ledger id.
///
/// Responds with 200 + `{ "serial_code": "...", "movements": [...] }`.
/// Empty `movements` array when the serial code does not exist.
pub async fn trace_serial_handler(
    State(state): State<Arc<AppState>>,
    Path((item_id, serial_code)): Path<(Uuid, String)>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    match trace_serial(&state.pool, &tenant_id, item_id, &serial_code).await {
        Ok(movements) => (
            StatusCode::OK,
            Json(json!({
                "serial_code": serial_code,
                "item_id": item_id,
                "movements": movements
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                error = %e,
                item_id = %item_id,
                serial_code = %serial_code,
                "database error tracing serial"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

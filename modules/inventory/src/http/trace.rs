//! Lot and serial traceability HTTP handlers.
//!
//! Endpoints:
//!   GET /api/inventory/items/{item_id}/lots/{lot_code}/trace
//!   GET /api/inventory/items/{item_id}/serials/{serial_code}/trace

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::lots_serials::queries::{trace_lot, trace_serial},
    AppState,
};

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/lots/{lot_code}/trace",
    tag = "Traceability",
    params(
        ("item_id" = Uuid, Path, description = "Item ID"),
        ("lot_code" = String, Path, description = "Lot code"),
    ),
    responses(
        (status = 200, description = "Lot traceability movements", body = serde_json::Value),
    ),
    security(("bearer" = [])),
)]
pub async fn trace_lot_handler(
    State(state): State<Arc<AppState>>,
    Path((item_id, lot_code)): Path<(Uuid, String)>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
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
            tracing::error!(error = %e, item_id = %item_id, lot_code = %lot_code, "database error tracing lot");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/serials/{serial_code}/trace",
    tag = "Traceability",
    params(
        ("item_id" = Uuid, Path, description = "Item ID"),
        ("serial_code" = String, Path, description = "Serial code"),
    ),
    responses(
        (status = 200, description = "Serial traceability movements", body = serde_json::Value),
    ),
    security(("bearer" = [])),
)]
pub async fn trace_serial_handler(
    State(state): State<Arc<AppState>>,
    Path((item_id, serial_code)): Path<(Uuid, String)>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
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
            tracing::error!(error = %e, item_id = %item_id, serial_code = %serial_code, "database error tracing serial");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

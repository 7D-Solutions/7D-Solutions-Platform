//! Inventory expiry HTTP handlers.

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
    domain::expiry::{
        run_expiry_alert_scan, set_lot_expiry, ExpiryError, RunExpiryAlertScanRequest,
        SetLotExpiryRequest,
    },
    AppState,
};

fn expiry_error_response(err: ExpiryError) -> impl IntoResponse {
    match err {
        ExpiryError::LotNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": err.to_string() })),
        ),
        ExpiryError::ExpiryDateRequired
        | ExpiryError::MissingShelfLifePolicy
        | ExpiryError::Validation(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": err.to_string() })),
        ),
        ExpiryError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "idempotency_conflict", "message": err.to_string() })),
        ),
        ExpiryError::Serialization(e) => {
            tracing::error!(error = %e, "expiry serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        ExpiryError::Database(e) => {
            tracing::error!(error = %e, "expiry database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// PUT /api/inventory/lots/:lot_id/expiry
pub async fn put_lot_expiry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(lot_id): Path<Uuid>,
    Json(mut req): Json<SetLotExpiryRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    req.lot_id = lot_id;

    match set_lot_expiry(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => expiry_error_response(e).into_response(),
    }
}

/// POST /api/inventory/expiry-alerts/scan
pub async fn post_expiry_alert_scan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<RunExpiryAlertScanRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match run_expiry_alert_scan(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => expiry_error_response(e).into_response(),
    }
}

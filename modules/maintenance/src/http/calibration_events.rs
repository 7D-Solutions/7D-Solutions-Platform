//! Calibration event HTTP handlers.
//!
//! Endpoints:
//!   POST /api/maintenance/assets/:asset_id/calibration-events  — record calibration
//!   GET  /api/maintenance/assets/:asset_id/calibration-status  — get derived status

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

use super::ErrorBody;
use crate::domain::calibration_events::{
    CalibrationEventError, CalibrationEventRepo, RecordCalibrationRequest,
};
use crate::AppState;

fn cal_event_error_response(err: CalibrationEventError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        CalibrationEventError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("asset_not_found", "Asset not found")),
        ),
        CalibrationEventError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        CalibrationEventError::IdempotentDuplicate(_) => (
            StatusCode::OK,
            Json(ErrorBody::new(
                "idempotent_duplicate",
                "Calibration event already exists",
            )),
        ),
        CalibrationEventError::Database(e) => {
            tracing::error!(error = %e, "calibration event database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/assets/:asset_id/calibration-events
pub async fn record_calibration_event(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<RecordCalibrationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;

    match CalibrationEventRepo::record(&state.pool, asset_id, &req).await {
        Ok(event) => (StatusCode::CREATED, Json(json!(event))).into_response(),
        Err(CalibrationEventError::IdempotentDuplicate(event)) => {
            (StatusCode::OK, Json(json!(event))).into_response()
        }
        Err(e) => cal_event_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets/:asset_id/calibration-status
pub async fn get_calibration_status(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match CalibrationEventRepo::get_status(&state.pool, asset_id, &tenant_id).await {
        Ok(status) => (StatusCode::OK, Json(json!(status))).into_response(),
        Err(e) => cal_event_error_response(e).into_response(),
    }
}

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
    }
}

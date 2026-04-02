use axum::{extract::{Path, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;
use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::calibration_events::{CalibrationEvent, CalibrationEventError, CalibrationEventRepo, CalibrationStatusResponse, RecordCalibrationRequest};
use crate::AppState;

#[utoipa::path(
    post, path = "/api/maintenance/assets/{asset_id}/calibration-events", tag = "Calibration",
    params(("asset_id" = Uuid, Path, description = "Asset ID")),
    request_body = RecordCalibrationRequest,
    responses(
        (status = 201, description = "Calibration event recorded", body = CalibrationEvent),
        (status = 200, description = "Idempotent duplicate", body = CalibrationEvent),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn record_calibration_event(State(state): State<Arc<AppState>>, Path(asset_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<RecordCalibrationRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match CalibrationEventRepo::record(&state.pool, asset_id, &req).await { Ok(ev) => (StatusCode::CREATED, Json(ev)).into_response(), Err(CalibrationEventError::IdempotentDuplicate(ev)) => (StatusCode::OK, Json(ev)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/assets/{asset_id}/calibration-status", tag = "Calibration",
    params(("asset_id" = Uuid, Path, description = "Asset ID")),
    responses(
        (status = 200, description = "Current calibration status", body = CalibrationStatusResponse),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_calibration_status(State(state): State<Arc<AppState>>, Path(asset_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match CalibrationEventRepo::get_status(&state.pool, asset_id, &tenant_id).await { Ok(s) => (StatusCode::OK, Json(s)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

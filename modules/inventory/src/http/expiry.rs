//! Inventory expiry HTTP handlers.

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

use super::tenant::with_request_id;
use crate::{
    domain::expiry::{
        run_expiry_alert_scan, set_lot_expiry, LotExpiryRecord, RunExpiryAlertScanRequest,
        RunExpiryAlertScanResult, SetLotExpiryRequest,
    },
    AppState,
};
use platform_sdk::extract_tenant;

#[utoipa::path(
    put,
    path = "/api/inventory/lots/{lot_id}/expiry",
    tag = "Expiry",
    params(("lot_id" = Uuid, Path, description = "Lot ID")),
    request_body = SetLotExpiryRequest,
    responses(
        (status = 200, description = "Lot expiry set", body = LotExpiryRecord),
        (status = 404, description = "Lot not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn put_lot_expiry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(lot_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<SetLotExpiryRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    req.lot_id = lot_id;

    match set_lot_expiry(&state.pool, &req).await {
        Ok((result, _)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/expiry-alerts/scan",
    tag = "Expiry",
    request_body = RunExpiryAlertScanRequest,
    responses(
        (status = 200, description = "Expiry alert scan completed", body = RunExpiryAlertScanResult),
    ),
    security(("bearer" = [])),
)]
pub async fn post_expiry_alert_scan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<RunExpiryAlertScanRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match run_expiry_alert_scan(&state.pool, &req).await {
        Ok((result, _)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

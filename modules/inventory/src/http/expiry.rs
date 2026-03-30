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

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::expiry::{run_expiry_alert_scan, set_lot_expiry, RunExpiryAlertScanRequest, SetLotExpiryRequest},
    AppState,
};

/// PUT /api/inventory/lots/:lot_id/expiry
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

/// POST /api/inventory/expiry-alerts/scan
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

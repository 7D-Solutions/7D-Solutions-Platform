//! Status transfer HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/status-transfers
//!     — move quantity between status buckets (available/quarantine/damaged)
//!
//! Idempotency:
//!   Callers MUST supply `idempotency_key` in the request body.
//!   Duplicate keys with the same body return 200 OK with the stored result.
//!   Duplicate keys with a different body return 409 Conflict.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::status::transfer_service::{
        process_status_transfer, StatusTransferRequest, StatusTransferResult,
    },
    AppState,
};

#[utoipa::path(
    post,
    path = "/api/inventory/status-transfers",
    tag = "Status Transfers",
    request_body = StatusTransferRequest,
    responses(
        (status = 201, description = "Status transfer created", body = StatusTransferResult),
        (status = 200, description = "Idempotency replay", body = StatusTransferResult),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_status_transfer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<StatusTransferRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match process_status_transfer(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(err) => {
            let api_err: ApiError = err.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

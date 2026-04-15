//! Reservation HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/reservations/reserve          — create a stock hold
//!   POST /api/inventory/reservations/release          — compensating release
//!   POST /api/inventory/reservations/{id}/fulfill     — fulfill reservation
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
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::{
        fulfill_service::{process_fulfill, FulfillRequest, FulfillResult},
        reservation_service::{
            process_release, process_reserve, ReleaseRequest, ReleaseResult, ReserveRequest,
            ReserveResult,
        },
    },
    AppState,
};
use platform_sdk::extract_tenant;

#[utoipa::path(
    post,
    path = "/api/inventory/reservations/reserve",
    tag = "Reservations",
    request_body = ReserveRequest,
    responses(
        (status = 201, description = "Stock reserved", body = ReserveResult),
        (status = 200, description = "Idempotency replay", body = ReserveResult),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
        (status = 422, description = "Insufficient available stock", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_reserve(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<ReserveRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
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
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/reservations/release",
    tag = "Reservations",
    request_body = ReleaseRequest,
    responses(
        (status = 200, description = "Reservation released", body = ReleaseResult),
        (status = 404, description = "Reservation not found", body = ApiError),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_release(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<ReleaseRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match process_release(&state.pool, &req).await {
        Ok((result, _)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/reservations/{id}/fulfill",
    tag = "Reservations",
    params(("id" = Uuid, Path, description = "Reservation ID")),
    request_body = FulfillRequest,
    responses(
        (status = 200, description = "Reservation fulfilled", body = FulfillResult),
        (status = 404, description = "Reservation not found", body = ApiError),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_fulfill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(reservation_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<FulfillRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    req.reservation_id = reservation_id;
    match process_fulfill(&state.pool, &req).await {
        Ok((result, _)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_reservations_module_compiles() {}
}

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::operations::{OperationInstance, OperationRepo},
    AppState,
};
use platform_sdk::extract_tenant;

/// POST /api/production/work-orders/:id/operations/initialize
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{id}/operations/initialize",
    tag = "Operations",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 201, description = "Operations initialized", body = Vec<OperationInstance>),
        (status = 404, description = "Work order not found", body = ApiError),
        (status = 409, description = "Already initialized", body = ApiError),
        (status = 422, description = "Work order not released or no routing", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn initialize_operations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(wo_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match OperationRepo::initialize(&state.pool, wo_id, &tenant_id, &corr, None).await {
        Ok(ops) => (StatusCode::CREATED, Json(ops)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/work-orders/:wo_id/operations/:op_id/start
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{wo_id}/operations/{op_id}/start",
    tag = "Operations",
    params(
        ("wo_id" = Uuid, Path, description = "Work order ID"),
        ("op_id" = Uuid, Path, description = "Operation ID"),
    ),
    responses(
        (status = 200, description = "Operation started", body = OperationInstance),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid transition or predecessor not complete", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn start_operation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((wo_id, op_id)): Path<(Uuid, Uuid)>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match OperationRepo::start(&state.pool, wo_id, op_id, &tenant_id, &corr, None).await {
        Ok(op) => (StatusCode::OK, Json(op)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/work-orders/:wo_id/operations/:op_id/complete
#[utoipa::path(
    post,
    path = "/api/production/work-orders/{wo_id}/operations/{op_id}/complete",
    tag = "Operations",
    params(
        ("wo_id" = Uuid, Path, description = "Work order ID"),
        ("op_id" = Uuid, Path, description = "Operation ID"),
    ),
    responses(
        (status = 200, description = "Operation completed", body = OperationInstance),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn complete_operation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((wo_id, op_id)): Path<(Uuid, Uuid)>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match OperationRepo::complete(&state.pool, wo_id, op_id, &tenant_id, &corr, None).await {
        Ok(op) => (StatusCode::OK, Json(op)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/work-orders/:id/operations
#[utoipa::path(
    get,
    path = "/api/production/work-orders/{id}/operations",
    tag = "Operations",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Operations for work order", body = PaginatedResponse<OperationInstance>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_operations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(wo_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match OperationRepo::list(&state.pool, wo_id, &tenant_id).await {
        Ok(ops) => {
            let total = ops.len() as i64;
            let resp = PaginatedResponse::new(ops, 1, total, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

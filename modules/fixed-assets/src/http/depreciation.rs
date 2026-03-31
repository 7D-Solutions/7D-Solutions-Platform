//! HTTP handlers for depreciation schedule generation and run execution.

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

use crate::domain::depreciation::{
    CreateRunRequest, DepreciationService, GenerateScheduleRequest,
};
use crate::AppState;

use super::helpers::tenant::{extract_tenant, with_request_id};

// ============================================================================
// Schedule endpoints
// ============================================================================

#[utoipa::path(
    post, path = "/api/fixed-assets/depreciation/schedule", tag = "Depreciation",
    request_body = GenerateScheduleRequest,
    responses((status = 201, description = "Schedule generated"), (status = 401, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn generate_schedule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<GenerateScheduleRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match DepreciationService::generate_schedule(&state.pool, &req).await {
        Ok(schedules) => (StatusCode::CREATED, Json(schedules)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Run endpoints
// ============================================================================

#[utoipa::path(
    post, path = "/api/fixed-assets/depreciation/runs", tag = "Depreciation",
    request_body = CreateRunRequest,
    responses((status = 201, description = "Run created"), (status = 401, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateRunRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match DepreciationService::run(&state.pool, &req).await {
        Ok(run) => (StatusCode::CREATED, Json(run)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/fixed-assets/depreciation/runs", tag = "Depreciation",
    responses((status = 200, description = "Run list", body = PaginatedResponse<crate::domain::depreciation::DepreciationRun>)),
    security(("bearer" = [])),
)]
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match DepreciationService::list_runs(&state.pool, &tenant_id).await {
        Ok(runs) => {
            let total = runs.len() as i64;
            let resp = PaginatedResponse::new(runs, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/fixed-assets/depreciation/runs/{id}", tag = "Depreciation",
    params(("id" = Uuid, Path, description = "Run ID")),
    responses((status = 200, description = "Run details"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match DepreciationService::get_run(&state.pool, id, &tenant_id).await {
        Ok(Some(run)) => Json(run).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Depreciation run {} not found", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

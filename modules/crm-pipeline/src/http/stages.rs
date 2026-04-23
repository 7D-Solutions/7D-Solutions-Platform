//! HTTP handlers for pipeline stage configuration — 5 endpoints per spec §4.3.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;

use crate::domain::pipeline_stages::{
    service, CreateStageRequest, ReorderStagesRequest, UpdateStageRequest,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    get, path = "/api/crm-pipeline/stages", tag = "PipelineStages",
    responses((status = 200, body = Vec<crate::domain::pipeline_stages::PipelineStage>)),
    security(("bearer" = [])),
)]
pub async fn list_stages(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::list_stages(&state.pool, &tenant_id).await {
        Ok(stages) => Json(stages).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/stages", tag = "PipelineStages",
    request_body = CreateStageRequest,
    responses((status = 201, body = crate::domain::pipeline_stages::PipelineStage), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_stage(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateStageRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match service::create_stage(&state.pool, &tenant_id, &req, &actor).await {
        Ok(stage) => (StatusCode::CREATED, Json(stage)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/crm-pipeline/stages/{code}", tag = "PipelineStages",
    request_body = UpdateStageRequest,
    responses((status = 200, body = crate::domain::pipeline_stages::PipelineStage)),
    security(("bearer" = [])),
)]
pub async fn update_stage(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(code): Path<String>,
    Json(req): Json<UpdateStageRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match service::update_stage(&state.pool, &tenant_id, &code, &req, &actor).await {
        Ok(stage) => Json(stage).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/stages/{code}/deactivate", tag = "PipelineStages",
    responses((status = 200, body = crate::domain::pipeline_stages::PipelineStage)),
    security(("bearer" = [])),
)]
pub async fn deactivate_stage(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match service::deactivate_stage(&state.pool, &tenant_id, &code, &actor).await {
        Ok(stage) => Json(stage).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/stages/reorder", tag = "PipelineStages",
    request_body = ReorderStagesRequest,
    responses((status = 200, body = Vec<crate::domain::pipeline_stages::PipelineStage>)),
    security(("bearer" = [])),
)]
pub async fn reorder_stages(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<ReorderStagesRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match service::reorder_stages(&state.pool, &tenant_id, &req, &actor).await {
        Ok(stages) => Json(stages).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

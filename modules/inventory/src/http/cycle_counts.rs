//! Cycle count task HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/cycle-count-tasks              — create a task + snapshot lines
//!   POST /api/inventory/cycle-count-tasks/{id}/submit  — submit counted quantities
//!   POST /api/inventory/cycle-count-tasks/{id}/approve — approve and apply adjustments

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::cycle_count::{
        approve_service::{approve_cycle_count, ApproveRequest},
        submit_service::{submit_cycle_count, SubmitLineInput, SubmitRequest},
        task_service::{create_cycle_count_task, CreateTaskRequest, CreateTaskResult},
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/cycle-count-tasks",
    tag = "Cycle Counts",
    request_body = CreateTaskRequest,
    responses(
        (status = 201, description = "Cycle count task created with snapshot lines", body = CreateTaskResult),
    ),
    security(("bearer" = [])),
)]
pub async fn post_cycle_count_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match create_cycle_count_task(&state.pool, &req).await {
        Ok(result) => (StatusCode::CREATED, Json(result)).into_response(),
        Err(err) => {
            let api_err: ApiError = err.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Submit
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SubmitBody {
    pub idempotency_key: String,
    #[serde(default)]
    pub lines: Vec<SubmitLineInput>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/inventory/cycle-count-tasks/{task_id}/submit",
    tag = "Cycle Counts",
    params(("task_id" = Uuid, Path, description = "Cycle count task ID")),
    request_body = SubmitBody,
    responses(
        (status = 201, description = "Count submitted", body = serde_json::Value),
        (status = 200, description = "Idempotency replay", body = serde_json::Value),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_cycle_count_submit(
    Path(task_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(body): Json<SubmitBody>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let req = SubmitRequest {
        task_id,
        tenant_id,
        idempotency_key: body.idempotency_key,
        lines: body.lines,
        correlation_id: body.correlation_id,
        causation_id: body.causation_id,
    };
    match submit_cycle_count(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => {
            let api_err: ApiError = err.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Approve
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ApproveBody {
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/inventory/cycle-count-tasks/{task_id}/approve",
    tag = "Cycle Counts",
    params(("task_id" = Uuid, Path, description = "Cycle count task ID")),
    request_body = ApproveBody,
    responses(
        (status = 201, description = "Count approved and adjustments applied", body = serde_json::Value),
        (status = 200, description = "Idempotency replay", body = serde_json::Value),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_cycle_count_approve(
    Path(task_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(body): Json<ApproveBody>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let req = ApproveRequest {
        task_id,
        tenant_id,
        idempotency_key: body.idempotency_key,
        correlation_id: body.correlation_id,
        causation_id: body.causation_id,
    };
    match approve_cycle_count(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => {
            let api_err: ApiError = err.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

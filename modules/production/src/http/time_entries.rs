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

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::time_entries::{
        ManualEntryRequest, StartTimerRequest, StopTimerRequest, TimeEntry, TimeEntryRepo,
    },
    AppState,
};

/// POST /api/production/time-entries/start
#[utoipa::path(
    post,
    path = "/api/production/time-entries/start",
    tag = "Time Entries",
    request_body = StartTimerRequest,
    responses(
        (status = 201, description = "Timer started", body = TimeEntry),
        (status = 404, description = "Work order not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn start_timer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<StartTimerRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match TimeEntryRepo::start_timer(&state.pool, &req, &tenant_id, &corr, None).await {
        Ok(entry) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/time-entries/:id/stop
#[utoipa::path(
    post,
    path = "/api/production/time-entries/{id}/stop",
    tag = "Time Entries",
    params(("id" = Uuid, Path, description = "Time entry ID")),
    request_body = StopTimerRequest,
    responses(
        (status = 200, description = "Timer stopped", body = TimeEntry),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Already stopped", body = ApiError),
        (status = 422, description = "Invalid time range", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn stop_timer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<StopTimerRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match TimeEntryRepo::stop_timer(&state.pool, id, &req, &tenant_id, &corr, None).await {
        Ok(entry) => (StatusCode::OK, Json(entry)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/time-entries/manual
#[utoipa::path(
    post,
    path = "/api/production/time-entries/manual",
    tag = "Time Entries",
    request_body = ManualEntryRequest,
    responses(
        (status = 201, description = "Manual entry created", body = TimeEntry),
        (status = 404, description = "Work order not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn manual_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<ManualEntryRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match TimeEntryRepo::manual_entry(&state.pool, &req, &tenant_id, &corr, None).await {
        Ok(entry) => (StatusCode::CREATED, Json(entry)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/work-orders/:id/time-entries
#[utoipa::path(
    get,
    path = "/api/production/work-orders/{id}/time-entries",
    tag = "Time Entries",
    params(("id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Time entries for work order", body = PaginatedResponse<TimeEntry>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_time_entries(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(wo_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match TimeEntryRepo::list_by_work_order(&state.pool, wo_id, &tenant_id).await {
        Ok(entries) => {
            let total = entries.len() as i64;
            let resp = PaginatedResponse::new(entries, 1, total, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

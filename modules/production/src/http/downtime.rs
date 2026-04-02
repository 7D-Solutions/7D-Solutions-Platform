use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::pagination::PaginationQuery;
use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::downtime::{DowntimeRepo, EndDowntimeRequest, StartDowntimeRequest, WorkcenterDowntime},
    AppState,
};

/// POST /api/production/workcenters/:id/downtime/start
#[utoipa::path(
    post,
    path = "/api/production/workcenters/{id}/downtime/start",
    tag = "Downtime",
    params(("id" = Uuid, Path, description = "Workcenter ID")),
    request_body = StartDowntimeRequest,
    responses(
        (status = 201, description = "Downtime started", body = WorkcenterDowntime),
        (status = 404, description = "Workcenter not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn start_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(workcenter_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<StartDowntimeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    req.workcenter_id = workcenter_id;
    let corr = Uuid::new_v4().to_string();
    match DowntimeRepo::start(&state.pool, &req, &corr, None).await {
        Ok(dt) => (StatusCode::CREATED, Json(dt)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/downtime/:id/end
#[utoipa::path(
    post,
    path = "/api/production/downtime/{id}/end",
    tag = "Downtime",
    params(("id" = Uuid, Path, description = "Downtime ID")),
    request_body = EndDowntimeRequest,
    responses(
        (status = 200, description = "Downtime ended", body = WorkcenterDowntime),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Already ended", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn end_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(downtime_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<EndDowntimeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match DowntimeRepo::end(&state.pool, downtime_id, &req, &corr, None).await {
        Ok(dt) => (StatusCode::OK, Json(dt)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/downtime/active
#[utoipa::path(
    get,
    path = "/api/production/downtime/active",
    tag = "Downtime",
    params(PaginationQuery),
    responses(
        (status = 200, description = "Active downtime records", body = PaginatedResponse<WorkcenterDowntime>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_active_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(pq): Query<PaginationQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = pq.page.max(1);
    let page_size = pq.page_size.clamp(1, 200);
    match DowntimeRepo::list_active(&state.pool, &tenant_id, page, page_size).await {
        Ok((items, total)) => {
            let resp = PaginatedResponse::new(items, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/workcenters/:id/downtime
#[utoipa::path(
    get,
    path = "/api/production/workcenters/{id}/downtime",
    tag = "Downtime",
    params(("id" = Uuid, Path, description = "Workcenter ID")),
    responses(
        (status = 200, description = "Downtime records for workcenter", body = PaginatedResponse<WorkcenterDowntime>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_workcenter_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(workcenter_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match DowntimeRepo::list_for_workcenter(&state.pool, workcenter_id, &tenant_id).await {
        Ok(list) => {
            let total = list.len() as i64;
            let resp = PaginatedResponse::new(list, 1, total, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

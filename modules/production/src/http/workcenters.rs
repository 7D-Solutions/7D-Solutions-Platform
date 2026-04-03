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
    domain::workcenters::{
        CreateWorkcenterRequest, UpdateWorkcenterRequest, Workcenter, WorkcenterRepo,
    },
    AppState,
};

/// POST /api/production/workcenters
#[utoipa::path(
    post,
    path = "/api/production/workcenters",
    tag = "Workcenters",
    request_body = CreateWorkcenterRequest,
    responses(
        (status = 201, description = "Workcenter created", body = Workcenter),
        (status = 409, description = "Duplicate code", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_workcenter(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateWorkcenterRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match WorkcenterRepo::create(&state.pool, &req, &corr, None).await {
        Ok(wc) => (StatusCode::CREATED, Json(wc)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/workcenters/:id
#[utoipa::path(
    get,
    path = "/api/production/workcenters/{id}",
    tag = "Workcenters",
    params(("id" = Uuid, Path, description = "Workcenter ID")),
    responses(
        (status = 200, description = "Workcenter details", body = Workcenter),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_workcenter(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match WorkcenterRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(wc)) => (StatusCode::OK, Json(wc)).into_response(),
        Ok(None) => {
            with_request_id(ApiError::not_found("Workcenter not found"), &tracing_ctx)
                .into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// Name filter for workcenter list.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WorkcenterNameFilter {
    /// Filter by name (case-insensitive substring match).
    pub name: Option<String>,
}

/// GET /api/production/workcenters
#[utoipa::path(
    get,
    path = "/api/production/workcenters",
    tag = "Workcenters",
    params(PaginationQuery, WorkcenterNameFilter),
    responses(
        (status = 200, description = "Paginated workcenter list", body = PaginatedResponse<Workcenter>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_workcenters(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(pq): Query<PaginationQuery>,
    Query(nq): Query<WorkcenterNameFilter>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = pq.page.max(1);
    let page_size = pq.page_size.clamp(1, 200);
    let name = nq.name.as_deref();
    match WorkcenterRepo::list(&state.pool, &tenant_id, page, page_size, name).await {
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

/// PUT /api/production/workcenters/:id
#[utoipa::path(
    put,
    path = "/api/production/workcenters/{id}",
    tag = "Workcenters",
    params(("id" = Uuid, Path, description = "Workcenter ID")),
    request_body = UpdateWorkcenterRequest,
    responses(
        (status = 200, description = "Workcenter updated", body = Workcenter),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_workcenter(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateWorkcenterRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match WorkcenterRepo::update(&state.pool, id, &req, &corr, None).await {
        Ok(wc) => (StatusCode::OK, Json(wc)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/workcenters/:id/deactivate
#[utoipa::path(
    post,
    path = "/api/production/workcenters/{id}/deactivate",
    tag = "Workcenters",
    params(("id" = Uuid, Path, description = "Workcenter ID")),
    responses(
        (status = 200, description = "Workcenter deactivated", body = Workcenter),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn deactivate_workcenter(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match WorkcenterRepo::deactivate(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(wc) => (StatusCode::OK, Json(wc)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

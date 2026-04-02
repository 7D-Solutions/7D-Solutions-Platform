use axum::{extract::{Path, Query, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::plans::{AssignPlanRequest, AssignmentRepo, CreatePlanRequest, ListAssignmentsQuery, ListPlansQuery, MaintenancePlan, PlanAssignment, PlanRepo, UpdatePlanRequest};
use crate::AppState;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListPlansParams {
    pub is_active: Option<bool>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListAssignmentsParams {
    pub plan_id: Option<Uuid>,
    pub asset_id: Option<Uuid>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    post, path = "/api/maintenance/plans", tag = "Plans",
    request_body = CreatePlanRequest,
    responses(
        (status = 201, description = "Plan created", body = MaintenancePlan),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_plan(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<CreatePlanRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match PlanRepo::create(&state.pool, &req).await { Ok(p) => (StatusCode::CREATED, Json(p)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/plans", tag = "Plans",
    params(ListPlansParams),
    responses(
        (status = 200, description = "Paginated maintenance plans", body = PaginatedResponse<MaintenancePlan>),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_plans(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, Query(params): Query<ListPlansParams>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    let page = params.page.unwrap_or(1).max(1); let page_size = params.page_size.unwrap_or(50).clamp(1, 200); let offset = (page - 1) * page_size;
    let q = ListPlansQuery { tenant_id, is_active: params.is_active, limit: Some(page_size), offset: Some(offset) };
    let total = match PlanRepo::count(&state.pool, &q).await { Ok(t) => t, Err(e) => { let a = ApiError::from(e); return with_request_id(a, &tracing_ctx).into_response(); } };
    match PlanRepo::list(&state.pool, &q).await { Ok(v) => (StatusCode::OK, Json(PaginatedResponse::new(v, page, page_size, total))).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/plans/{plan_id}", tag = "Plans",
    params(("plan_id" = Uuid, Path, description = "Plan ID")),
    responses(
        (status = 200, description = "Plan details", body = MaintenancePlan),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_plan(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match PlanRepo::find_by_id(&state.pool, id, &tenant_id).await { Ok(Some(p)) => (StatusCode::OK, Json(p)).into_response(), Ok(None) => with_request_id(ApiError::not_found("Plan not found"), &tracing_ctx).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    patch, path = "/api/maintenance/plans/{plan_id}", tag = "Plans",
    params(("plan_id" = Uuid, Path, description = "Plan ID")),
    request_body = UpdatePlanRequest,
    responses(
        (status = 200, description = "Plan updated", body = MaintenancePlan),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_plan(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(req): Json<UpdatePlanRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match PlanRepo::update(&state.pool, id, &tenant_id, &req).await { Ok(p) => (StatusCode::OK, Json(p)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    post, path = "/api/maintenance/plans/{plan_id}/assign", tag = "Plans",
    params(("plan_id" = Uuid, Path, description = "Plan ID")),
    request_body = AssignPlanRequest,
    responses(
        (status = 201, description = "Plan assigned to asset", body = PlanAssignment),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn assign_plan(State(state): State<Arc<AppState>>, Path(plan_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<AssignPlanRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match AssignmentRepo::assign(&state.pool, plan_id, &req).await { Ok(a) => (StatusCode::CREATED, Json(a)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/assignments", tag = "Plans",
    params(ListAssignmentsParams),
    responses(
        (status = 200, description = "Paginated plan assignments", body = PaginatedResponse<PlanAssignment>),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_assignments(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, Query(params): Query<ListAssignmentsParams>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    let page = params.page.unwrap_or(1).max(1); let page_size = params.page_size.unwrap_or(50).clamp(1, 200); let offset = (page - 1) * page_size;
    let q = ListAssignmentsQuery { tenant_id, plan_id: params.plan_id, asset_id: params.asset_id, limit: Some(page_size), offset: Some(offset) };
    let total = match AssignmentRepo::count(&state.pool, &q).await { Ok(t) => t, Err(e) => { let a = ApiError::from(e); return with_request_id(a, &tracing_ctx).into_response(); } };
    match AssignmentRepo::list(&state.pool, &q).await { Ok(v) => (StatusCode::OK, Json(PaginatedResponse::new(v, page, page_size, total))).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

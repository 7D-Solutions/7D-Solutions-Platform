//! Allocation and rollup HTTP handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::NaiveDate;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::allocations::{
        models::{
            Allocation, AllocationError, CreateAllocationRequest, EmployeeRollup, ProjectRollup,
            TaskRollup, UpdateAllocationRequest,
        },
        service,
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AllocationListQuery {
    pub employee_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    #[serde(default = "default_true")]
    pub active_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct RollupQuery {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

fn default_true() -> bool {
    true
}

// ============================================================================
// Error mapping
// ============================================================================

fn map_allocation_error(err: AllocationError) -> ApiError {
    match err {
        AllocationError::NotFound => ApiError::not_found("Allocation not found"),
        AllocationError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        AllocationError::Database(e) => ApiError::internal(e.to_string()),
    }
}

// ============================================================================
// Allocation CRUD handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/allocations",
    request_body = CreateAllocationRequest,
    responses(
        (status = 201, description = "Allocation created", body = Allocation),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Allocations",
)]
pub async fn create_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateAllocationRequest>,
) -> Result<(StatusCode, Json<Allocation>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let alloc = service::create_allocation(&state.pool, &req)
        .await
        .map_err(map_allocation_error)?;
    Ok((StatusCode::CREATED, Json(alloc)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/allocations",
    params(
        ("employee_id" = Option<Uuid>, Query, description = "Filter by employee"),
        ("project_id" = Option<Uuid>, Query, description = "Filter by project"),
        ("active_only" = bool, Query, description = "Active only (default true)"),
    ),
    responses(
        (status = 200, description = "Allocation list", body = Vec<Allocation>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Allocations",
)]
pub async fn list_allocations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<AllocationListQuery>,
) -> Result<Json<PaginatedResponse<Allocation>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let allocs = service::list_allocations(
        &state.pool,
        &app_id,
        q.employee_id,
        q.project_id,
        q.active_only,
    )
    .await
    .map_err(map_allocation_error)?;
    let total = allocs.len() as i64;
    Ok(Json(PaginatedResponse::new(allocs, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/allocations/{id}",
    params(("id" = Uuid, Path, description = "Allocation UUID")),
    responses(
        (status = 200, description = "Allocation found", body = Allocation),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Allocations",
)]
pub async fn get_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Allocation>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let alloc = service::get_allocation(&state.pool, id, &app_id)
        .await
        .map_err(map_allocation_error)?;
    Ok(Json(alloc))
}

#[utoipa::path(
    put,
    path = "/api/timekeeping/allocations/{id}",
    params(("id" = Uuid, Path, description = "Allocation UUID")),
    request_body = UpdateAllocationRequest,
    responses(
        (status = 200, description = "Allocation updated", body = Allocation),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Allocations",
)]
pub async fn update_allocation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAllocationRequest>,
) -> Result<Json<Allocation>, ApiError> {
    let alloc = service::update_allocation(&state.pool, id, &req)
        .await
        .map_err(map_allocation_error)?;
    Ok(Json(alloc))
}

#[utoipa::path(
    delete,
    path = "/api/timekeeping/allocations/{id}",
    params(("id" = Uuid, Path, description = "Allocation UUID")),
    responses(
        (status = 200, description = "Allocation deactivated", body = Allocation),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Allocations",
)]
pub async fn deactivate_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Allocation>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let alloc = service::deactivate_allocation(&state.pool, id, &app_id)
        .await
        .map_err(map_allocation_error)?;
    Ok(Json(alloc))
}

// ============================================================================
// Rollup handlers (actual time from entries)
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/timekeeping/rollups/by-project",
    params(
        ("from" = NaiveDate, Query, description = "Period start date"),
        ("to" = NaiveDate, Query, description = "Period end date"),
    ),
    responses(
        (status = 200, description = "Project rollups", body = Vec<ProjectRollup>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Rollups",
)]
pub async fn rollup_by_project(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<RollupQuery>,
) -> Result<Json<PaginatedResponse<ProjectRollup>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let rows = service::rollup_by_project(&state.pool, &app_id, q.from, q.to)
        .await
        .map_err(map_allocation_error)?;
    let total = rows.len() as i64;
    Ok(Json(PaginatedResponse::new(rows, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/rollups/by-employee",
    params(
        ("from" = NaiveDate, Query, description = "Period start date"),
        ("to" = NaiveDate, Query, description = "Period end date"),
    ),
    responses(
        (status = 200, description = "Employee rollups", body = Vec<EmployeeRollup>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Rollups",
)]
pub async fn rollup_by_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<RollupQuery>,
) -> Result<Json<PaginatedResponse<EmployeeRollup>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let rows = service::rollup_by_employee(&state.pool, &app_id, q.from, q.to)
        .await
        .map_err(map_allocation_error)?;
    let total = rows.len() as i64;
    Ok(Json(PaginatedResponse::new(rows, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/rollups/by-task/{project_id}",
    params(
        ("project_id" = Uuid, Path, description = "Project UUID"),
        ("from" = NaiveDate, Query, description = "Period start date"),
        ("to" = NaiveDate, Query, description = "Period end date"),
    ),
    responses(
        (status = 200, description = "Task rollups for project", body = Vec<TaskRollup>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Rollups",
)]
pub async fn rollup_by_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<RollupQuery>,
) -> Result<Json<PaginatedResponse<TaskRollup>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let rows = service::rollup_by_task(&state.pool, &app_id, project_id, q.from, q.to)
        .await
        .map_err(map_allocation_error)?;
    let total = rows.len() as i64;
    Ok(Json(PaginatedResponse::new(rows, 1, total, total)))
}

//! Employee HTTP handlers — CRUD endpoints for the employee directory.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::employees::{
        models::{CreateEmployeeRequest, Employee, EmployeeError, UpdateEmployeeRequest},
        service::EmployeeRepo,
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Error mapping
// ============================================================================

fn map_employee_error(err: EmployeeError) -> ApiError {
    match err {
        EmployeeError::NotFound => ApiError::not_found("Employee not found"),
        EmployeeError::DuplicateCode(code, app) => ApiError::conflict(format!(
            "Employee code '{}' already exists for app '{}'",
            code, app
        )),
        EmployeeError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        EmployeeError::Database(e) => ApiError::internal(e.to_string()),
    }
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/employees",
    request_body = CreateEmployeeRequest,
    responses(
        (status = 201, description = "Employee created", body = Employee),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 409, description = "Duplicate employee code", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Employees",
)]
pub async fn create_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateEmployeeRequest>,
) -> Result<(StatusCode, Json<Employee>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let emp = EmployeeRepo::create(&state.pool, &req)
        .await
        .map_err(map_employee_error)?;
    Ok((StatusCode::CREATED, Json(emp)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/employees/{id}",
    params(("id" = Uuid, Path, description = "Employee UUID")),
    responses(
        (status = 200, description = "Employee found", body = Employee),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Employees",
)]
pub async fn get_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Employee>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let emp = EmployeeRepo::find_by_id(&state.pool, id, &app_id)
        .await
        .map_err(map_employee_error)?
        .ok_or_else(|| ApiError::not_found("Employee not found"))?;
    Ok(Json(emp))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/employees",
    responses(
        (status = 200, description = "Employee list", body = Vec<Employee>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Employees",
)]
pub async fn list_employees(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<PaginatedResponse<Employee>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let employees = EmployeeRepo::list(&state.pool, &app_id, true)
        .await
        .map_err(map_employee_error)?;
    let total = employees.len() as i64;
    Ok(Json(PaginatedResponse::new(employees, 1, total, total)))
}

#[utoipa::path(
    put,
    path = "/api/timekeeping/employees/{id}",
    params(("id" = Uuid, Path, description = "Employee UUID")),
    request_body = UpdateEmployeeRequest,
    responses(
        (status = 200, description = "Employee updated", body = Employee),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Employees",
)]
pub async fn update_employee(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEmployeeRequest>,
) -> Result<Json<Employee>, ApiError> {
    let emp = EmployeeRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_employee_error)?;
    Ok(Json(emp))
}

#[utoipa::path(
    delete,
    path = "/api/timekeeping/employees/{id}",
    params(("id" = Uuid, Path, description = "Employee UUID")),
    responses(
        (status = 200, description = "Employee deactivated", body = Employee),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Employees",
)]
pub async fn deactivate_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Employee>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let emp = EmployeeRepo::deactivate(&state.pool, id, &app_id)
        .await
        .map_err(map_employee_error)?;
    Ok(Json(emp))
}

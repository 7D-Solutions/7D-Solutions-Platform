//! Employee HTTP handlers — CRUD endpoints for the employee directory.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::employees::{
        models::{CreateEmployeeRequest, EmployeeError, UpdateEmployeeRequest},
        service::EmployeeRepo,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn employee_error_response(err: EmployeeError) -> impl IntoResponse {
    match err {
        EmployeeError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Employee not found" })),
        ),
        EmployeeError::DuplicateCode(code, app) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_code",
                "message": format!("Employee code '{}' already exists for app '{}'", code, app)
            })),
        ),
        EmployeeError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        EmployeeError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/timekeeping/employees
pub async fn create_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateEmployeeRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match EmployeeRepo::create(&state.pool, &req).await {
        Ok(emp) => (StatusCode::CREATED, Json(json!(emp))).into_response(),
        Err(err) => employee_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/employees/:id
pub async fn get_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match EmployeeRepo::find_by_id(&state.pool, id, &app_id).await {
        Ok(Some(emp)) => (StatusCode::OK, Json(json!(emp))).into_response(),
        Ok(None) => employee_error_response(EmployeeError::NotFound).into_response(),
        Err(err) => employee_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/employees
pub async fn list_employees(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match EmployeeRepo::list(&state.pool, &app_id, true).await {
        Ok(employees) => (StatusCode::OK, Json(json!(employees))).into_response(),
        Err(err) => employee_error_response(err).into_response(),
    }
}

/// PUT /api/timekeeping/employees/:id
pub async fn update_employee(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEmployeeRequest>,
) -> impl IntoResponse {
    match EmployeeRepo::update(&state.pool, id, &req).await {
        Ok(emp) => (StatusCode::OK, Json(json!(emp))).into_response(),
        Err(err) => employee_error_response(err).into_response(),
    }
}

/// DELETE /api/timekeeping/employees/:id
pub async fn deactivate_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match EmployeeRepo::deactivate(&state.pool, id, &app_id).await {
        Ok(emp) => (StatusCode::OK, Json(json!(emp))).into_response(),
        Err(err) => employee_error_response(err).into_response(),
    }
}

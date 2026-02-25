//! Allocation and rollup HTTP handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::allocations::{
        models::{AllocationError, CreateAllocationRequest, UpdateAllocationRequest},
        service,
    },
    AppState,
};

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

fn allocation_error_response(err: AllocationError) -> impl IntoResponse {
    match err {
        AllocationError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Allocation not found" })),
        ),
        AllocationError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        AllocationError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Allocation CRUD handlers
// ============================================================================

/// POST /api/timekeeping/allocations
pub async fn create_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateAllocationRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::create_allocation(&state.pool, &req).await {
        Ok(alloc) => (StatusCode::CREATED, Json(json!(alloc))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/allocations
pub async fn list_allocations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<AllocationListQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_allocations(
        &state.pool,
        &app_id,
        q.employee_id,
        q.project_id,
        q.active_only,
    )
    .await
    {
        Ok(allocs) => (StatusCode::OK, Json(json!(allocs))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/allocations/:id
pub async fn get_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::get_allocation(&state.pool, id, &app_id).await {
        Ok(alloc) => (StatusCode::OK, Json(json!(alloc))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

/// PUT /api/timekeeping/allocations/:id
pub async fn update_allocation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAllocationRequest>,
) -> impl IntoResponse {
    match service::update_allocation(&state.pool, id, &req).await {
        Ok(alloc) => (StatusCode::OK, Json(json!(alloc))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

/// DELETE /api/timekeeping/allocations/:id
pub async fn deactivate_allocation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::deactivate_allocation(&state.pool, id, &app_id).await {
        Ok(alloc) => (StatusCode::OK, Json(json!(alloc))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

// ============================================================================
// Rollup handlers (actual time from entries)
// ============================================================================

/// GET /api/timekeeping/rollups/by-project
pub async fn rollup_by_project(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<RollupQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::rollup_by_project(&state.pool, &app_id, q.from, q.to).await {
        Ok(rows) => (StatusCode::OK, Json(json!(rows))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/rollups/by-employee
pub async fn rollup_by_employee(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<RollupQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::rollup_by_employee(&state.pool, &app_id, q.from, q.to).await {
        Ok(rows) => (StatusCode::OK, Json(json!(rows))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/rollups/by-task/:project_id
pub async fn rollup_by_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<RollupQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::rollup_by_task(&state.pool, &app_id, project_id, q.from, q.to).await {
        Ok(rows) => (StatusCode::OK, Json(json!(rows))).into_response(),
        Err(err) => allocation_error_response(err).into_response(),
    }
}

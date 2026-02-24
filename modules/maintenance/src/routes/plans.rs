//! Maintenance plan and assignment HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/maintenance/plans             — create plan
//!   GET   /api/maintenance/plans             — list plans
//!   GET   /api/maintenance/plans/:id         — get plan detail
//!   PATCH /api/maintenance/plans/:id         — update plan
//!   POST  /api/maintenance/plans/:id/assign  — assign plan to asset
//!   GET   /api/maintenance/assignments       — list assignments

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::plans::{
    AssignPlanRequest, AssignmentRepo, CreatePlanRequest, ListAssignmentsQuery,
    ListPlansQuery, PlanError, PlanRepo, UpdatePlanRequest,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct TenantQuery {
    pub tenant_id: String,
}

fn plan_error_response(err: PlanError) -> impl IntoResponse {
    match err {
        PlanError::PlanNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Plan not found" })),
        ),
        PlanError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Asset not found" })),
        ),
        PlanError::MeterTypeNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Meter type not found" })),
        ),
        PlanError::DuplicateAssignment => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_assignment",
                "message": "This plan is already assigned to this asset"
            })),
        ),
        PlanError::AssignmentNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Assignment not found" })),
        ),
        PlanError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        PlanError::Database(e) => {
            tracing::error!(error = %e, "plan database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/maintenance/plans
pub async fn create_plan(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreatePlanRequest>,
) -> impl IntoResponse {
    match PlanRepo::create(&state.pool, &req).await {
        Ok(plan) => (StatusCode::CREATED, Json(json!(plan))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/plans
pub async fn list_plans(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListPlansQuery>,
) -> impl IntoResponse {
    match PlanRepo::list(&state.pool, &q).await {
        Ok(plans) => (StatusCode::OK, Json(json!(plans))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/plans/:id
pub async fn get_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match PlanRepo::find_by_id(&state.pool, id, &q.tenant_id).await {
        Ok(Some(plan)) => (StatusCode::OK, Json(json!(plan))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Plan not found" })),
        )
            .into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// PATCH /api/maintenance/plans/:id
pub async fn update_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
    Json(req): Json<UpdatePlanRequest>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match PlanRepo::update(&state.pool, id, &q.tenant_id, &req).await {
        Ok(plan) => (StatusCode::OK, Json(json!(plan))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// POST /api/maintenance/plans/:id/assign
pub async fn assign_plan(
    State(state): State<Arc<AppState>>,
    Path(plan_id): Path<Uuid>,
    Json(req): Json<AssignPlanRequest>,
) -> impl IntoResponse {
    match AssignmentRepo::assign(&state.pool, plan_id, &req).await {
        Ok(assignment) => (StatusCode::CREATED, Json(json!(assignment))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assignments
pub async fn list_assignments(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListAssignmentsQuery>,
) -> impl IntoResponse {
    match AssignmentRepo::list(&state.pool, &q).await {
        Ok(assignments) => (StatusCode::OK, Json(json!(assignments))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

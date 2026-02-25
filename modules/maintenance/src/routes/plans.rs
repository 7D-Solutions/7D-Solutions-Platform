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
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::plans::{
    AssignPlanRequest, AssignmentRepo, CreatePlanRequest, ListAssignmentsQuery,
    ListPlansQuery, PlanError, PlanRepo, UpdatePlanRequest,
};
use crate::AppState;
use super::ErrorBody;

#[derive(Debug, Deserialize)]
pub struct ListPlansParams {
    pub is_active: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ListAssignmentsParams {
    pub plan_id: Option<Uuid>,
    pub asset_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn plan_error_response(err: PlanError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        PlanError::PlanNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Plan not found")),
        ),
        PlanError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Asset not found")),
        ),
        PlanError::MeterTypeNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Meter type not found")),
        ),
        PlanError::DuplicateAssignment => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_assignment",
                "This plan is already assigned to this asset",
            )),
        ),
        PlanError::AssignmentNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Assignment not found")),
        ),
        PlanError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        PlanError::Database(e) => {
            tracing::error!(error = %e, "plan database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/plans
pub async fn create_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreatePlanRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match PlanRepo::create(&state.pool, &req).await {
        Ok(plan) => (StatusCode::CREATED, Json(json!(plan))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/plans
pub async fn list_plans(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListPlansParams>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListPlansQuery {
        tenant_id,
        is_active: params.is_active,
        limit: params.limit,
        offset: params.offset,
    };
    match PlanRepo::list(&state.pool, &q).await {
        Ok(plans) => (StatusCode::OK, Json(json!(plans))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/plans/:id
pub async fn get_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match PlanRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(plan)) => (StatusCode::OK, Json(json!(plan))).into_response(),
        Ok(None) => plan_error_response(PlanError::PlanNotFound).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// PATCH /api/maintenance/plans/:id
pub async fn update_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<UpdatePlanRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match PlanRepo::update(&state.pool, id, &tenant_id, &req).await {
        Ok(plan) => (StatusCode::OK, Json(json!(plan))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// POST /api/maintenance/plans/:id/assign
pub async fn assign_plan(
    State(state): State<Arc<AppState>>,
    Path(plan_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<AssignPlanRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match AssignmentRepo::assign(&state.pool, plan_id, &req).await {
        Ok(assignment) => (StatusCode::CREATED, Json(json!(assignment))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assignments
pub async fn list_assignments(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListAssignmentsParams>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListAssignmentsQuery {
        tenant_id,
        plan_id: params.plan_id,
        asset_id: params.asset_id,
        limit: params.limit,
        offset: params.offset,
    };
    match AssignmentRepo::list(&state.pool, &q).await {
        Ok(assignments) => (StatusCode::OK, Json(json!(assignments))).into_response(),
        Err(e) => plan_error_response(e).into_response(),
    }
}

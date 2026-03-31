//! Approval workflow HTTP handlers — submit, approve, reject, recall, list.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::NaiveDate;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::approvals::{
        models::{
            ApprovalAction, ApprovalError, ApprovalRequest, RecallApprovalRequest,
            ReviewApprovalRequest, SubmitApprovalRequest,
        },
        service,
    },
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListApprovalsQuery {
    pub employee_id: Uuid,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

// ============================================================================
// Sub-collection wrapper
// ============================================================================

#[derive(Debug, Serialize)]
pub struct DataWrapper<T: Serialize> {
    pub data: Vec<T>,
}

// ============================================================================
// Error mapping
// ============================================================================

fn map_approval_error(err: ApprovalError) -> ApiError {
    match err {
        ApprovalError::NotFound => ApiError::not_found("Approval request not found"),
        ApprovalError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        ApprovalError::InvalidTransition { from, to } => {
            ApiError::conflict(format!("Cannot transition from {} to {}", from, to))
        }
        ApprovalError::Duplicate => {
            ApiError::conflict("Approval request already exists for this period")
        }
        ApprovalError::Database(e) => ApiError::internal(e.to_string()),
    }
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/approvals/submit",
    request_body = SubmitApprovalRequest,
    responses(
        (status = 200, description = "Approval submitted", body = ApprovalRequest),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 409, description = "Duplicate or invalid transition", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn submit_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<SubmitApprovalRequest>,
) -> Result<Json<ApprovalRequest>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let approval = service::submit(&state.pool, &req)
        .await
        .map_err(map_approval_error)?;
    Ok(Json(approval))
}

#[utoipa::path(
    post,
    path = "/api/timekeeping/approvals/approve",
    request_body = ReviewApprovalRequest,
    responses(
        (status = 200, description = "Approval approved", body = ApprovalRequest),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn approve_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<ReviewApprovalRequest>,
) -> Result<Json<ApprovalRequest>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let approval = service::approve(&state.pool, &req)
        .await
        .map_err(map_approval_error)?;
    Ok(Json(approval))
}

#[utoipa::path(
    post,
    path = "/api/timekeeping/approvals/reject",
    request_body = ReviewApprovalRequest,
    responses(
        (status = 200, description = "Approval rejected", body = ApprovalRequest),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn reject_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<ReviewApprovalRequest>,
) -> Result<Json<ApprovalRequest>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let approval = service::reject(&state.pool, &req)
        .await
        .map_err(map_approval_error)?;
    Ok(Json(approval))
}

#[utoipa::path(
    post,
    path = "/api/timekeeping/approvals/recall",
    request_body = RecallApprovalRequest,
    responses(
        (status = 200, description = "Approval recalled", body = ApprovalRequest),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn recall_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<RecallApprovalRequest>,
) -> Result<Json<ApprovalRequest>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let approval = service::recall(&state.pool, &req)
        .await
        .map_err(map_approval_error)?;
    Ok(Json(approval))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/approvals",
    params(
        ("employee_id" = Uuid, Query, description = "Employee UUID"),
        ("from" = NaiveDate, Query, description = "Period start date"),
        ("to" = NaiveDate, Query, description = "Period end date"),
    ),
    responses(
        (status = 200, description = "Approval list", body = Vec<ApprovalRequest>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn list_approvals(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListApprovalsQuery>,
) -> Result<Json<PaginatedResponse<ApprovalRequest>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let approvals = service::list_approvals(&state.pool, &app_id, q.employee_id, q.from, q.to)
        .await
        .map_err(map_approval_error)?;
    let total = approvals.len() as i64;
    Ok(Json(PaginatedResponse::new(approvals, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/approvals/pending",
    responses(
        (status = 200, description = "Pending approvals", body = Vec<ApprovalRequest>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn list_pending(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<PaginatedResponse<ApprovalRequest>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let approvals = service::list_pending_review(&state.pool, &app_id)
        .await
        .map_err(map_approval_error)?;
    let total = approvals.len() as i64;
    Ok(Json(PaginatedResponse::new(approvals, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/approvals/{id}",
    params(("id" = Uuid, Path, description = "Approval UUID")),
    responses(
        (status = 200, description = "Approval found", body = ApprovalRequest),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn get_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApprovalRequest>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let approval = service::get_approval(&state.pool, &app_id, id)
        .await
        .map_err(map_approval_error)?;
    Ok(Json(approval))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/approvals/{id}/actions",
    params(("id" = Uuid, Path, description = "Approval UUID")),
    responses(
        (status = 200, description = "Approval action history", body = Vec<ApprovalAction>),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Approvals",
)]
pub async fn approval_actions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DataWrapper<ApprovalAction>>, ApiError> {
    let actions = service::approval_actions(&state.pool, id)
        .await
        .map_err(map_approval_error)?;
    Ok(Json(DataWrapper { data: actions }))
}

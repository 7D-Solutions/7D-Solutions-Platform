//! Approval workflow HTTP handlers — submit, approve, reject, recall, list.

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
    domain::approvals::{
        models::{
            ApprovalError, RecallApprovalRequest, ReviewApprovalRequest, SubmitApprovalRequest,
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
// Error mapping
// ============================================================================

fn approval_error_response(err: ApprovalError) -> impl IntoResponse {
    match err {
        ApprovalError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Approval request not found" })),
        ),
        ApprovalError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ApprovalError::InvalidTransition { from, to } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "invalid_transition",
                "message": format!("Cannot transition from {} to {}", from, to)
            })),
        ),
        ApprovalError::Duplicate => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate",
                "message": "Approval request already exists for this period"
            })),
        ),
        ApprovalError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/timekeeping/approvals/submit
pub async fn submit_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<SubmitApprovalRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::submit(&state.pool, &req).await {
        Ok(approval) => (StatusCode::OK, Json(json!(approval))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/approvals/approve
pub async fn approve_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<ReviewApprovalRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::approve(&state.pool, &req).await {
        Ok(approval) => (StatusCode::OK, Json(json!(approval))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/approvals/reject
pub async fn reject_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<ReviewApprovalRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::reject(&state.pool, &req).await {
        Ok(approval) => (StatusCode::OK, Json(json!(approval))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/approvals/recall
pub async fn recall_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<RecallApprovalRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::recall(&state.pool, &req).await {
        Ok(approval) => (StatusCode::OK, Json(json!(approval))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/approvals
pub async fn list_approvals(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListApprovalsQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_approvals(&state.pool, &app_id, q.employee_id, q.from, q.to).await {
        Ok(approvals) => (StatusCode::OK, Json(json!(approvals))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/approvals/pending
pub async fn list_pending(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_pending_review(&state.pool, &app_id).await {
        Ok(approvals) => (StatusCode::OK, Json(json!(approvals))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/approvals/:id
pub async fn get_approval(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::get_approval(&state.pool, &app_id, id).await {
        Ok(approval) => (StatusCode::OK, Json(json!(approval))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/approvals/:id/actions
pub async fn approval_actions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match service::approval_actions(&state.pool, id).await {
        Ok(actions) => (StatusCode::OK, Json(json!(actions))).into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

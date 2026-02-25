//! Form submission HTTP handlers.
//!
//! Endpoints:
//!   POST /api/pdf/forms/submissions           — create draft
//!   PUT  /api/pdf/forms/submissions/:id       — autosave field_data
//!   POST /api/pdf/forms/submissions/:id/submit — validate and submit
//!   GET  /api/pdf/forms/submissions/:id       — get submission
//!   GET  /api/pdf/forms/submissions           — list submissions

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::submissions::{
    AutosaveRequest, CreateSubmissionRequest, ListSubmissionsQuery, SubmissionError,
    SubmissionRepo,
};

use super::templates::extract_tenant;

#[derive(Debug, Deserialize)]
pub struct ListSubmissionsParams {
    pub template_id: Option<Uuid>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn submission_error_response(err: SubmissionError) -> impl IntoResponse {
    match err {
        SubmissionError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Submission not found" })),
        ),
        SubmissionError::TemplateNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Template not found" })),
        ),
        SubmissionError::AlreadySubmitted => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "already_submitted", "message": "Submission has already been submitted" })),
        ),
        SubmissionError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        SubmissionError::Database(e) => {
            tracing::error!(error = %e, "submission database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/pdf/forms/submissions
pub async fn create_submission(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateSubmissionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match SubmissionRepo::create(&pool, &req).await {
        Ok(sub) => (StatusCode::CREATED, Json(json!(sub))).into_response(),
        Err(e) => submission_error_response(e).into_response(),
    }
}

/// PUT /api/pdf/forms/submissions/:id
pub async fn autosave_submission(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<AutosaveRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match SubmissionRepo::autosave(&pool, id, &tenant_id, &req).await {
        Ok(sub) => (StatusCode::OK, Json(json!(sub))).into_response(),
        Err(e) => submission_error_response(e).into_response(),
    }
}

/// POST /api/pdf/forms/submissions/:id/submit
pub async fn submit_submission(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match SubmissionRepo::submit(&pool, id, &tenant_id).await {
        Ok(sub) => (StatusCode::OK, Json(json!(sub))).into_response(),
        Err(e) => submission_error_response(e).into_response(),
    }
}

/// GET /api/pdf/forms/submissions/:id
pub async fn get_submission(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match SubmissionRepo::find_by_id(&pool, id, &tenant_id).await {
        Ok(Some(sub)) => (StatusCode::OK, Json(json!(sub))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Submission not found" })),
        )
            .into_response(),
        Err(e) => submission_error_response(e).into_response(),
    }
}

/// GET /api/pdf/forms/submissions
pub async fn list_submissions(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListSubmissionsParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListSubmissionsQuery {
        tenant_id,
        template_id: params.template_id,
        status: params.status,
        limit: params.limit,
        offset: params.offset,
    };
    match SubmissionRepo::list(&pool, &q).await {
        Ok(list) => (StatusCode::OK, Json(json!(list))).into_response(),
        Err(e) => submission_error_response(e).into_response(),
    }
}

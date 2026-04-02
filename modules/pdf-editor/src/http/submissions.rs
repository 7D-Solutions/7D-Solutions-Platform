//! Form submission HTTP handlers.
//!
//! Endpoints:
//!   POST /api/pdf/forms/submissions           — create draft
//!   PUT  /api/pdf/forms/submissions/:id       — autosave field_data
//!   POST /api/pdf/forms/submissions/:id/submit — validate and submit
//!   GET  /api/pdf/forms/submissions/:id       — get submission
//!   GET  /api/pdf/forms/submissions           — list submissions (paginated)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use sqlx::PgPool;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::domain::submissions::{
    AutosaveRequest, CreateSubmissionRequest, FormSubmission, ListSubmissionsQuery, SubmissionRepo,
};

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListSubmissionsParams {
    pub template_id: Option<Uuid>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// POST /api/pdf/forms/submissions
#[utoipa::path(
    post, path = "/api/pdf/forms/submissions", tag = "Submissions",
    request_body = CreateSubmissionRequest,
    responses(
        (status = 201, description = "Submission created", body = FormSubmission),
        (status = 400, body = ApiError), (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_submission(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateSubmissionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;
    let sub = SubmissionRepo::create(&pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;
    Ok((StatusCode::CREATED, Json(sub)))
}

/// PUT /api/pdf/forms/submissions/:id
#[utoipa::path(
    put, path = "/api/pdf/forms/submissions/{id}", tag = "Submissions",
    params(("id" = Uuid, Path)),
    request_body = AutosaveRequest,
    responses(
        (status = 200, description = "Submission autosaved", body = FormSubmission),
        (status = 404, body = ApiError), (status = 409, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn autosave_submission(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<AutosaveRequest>,
) -> Result<Json<FormSubmission>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let sub = SubmissionRepo::autosave(&pool, id, &tenant_id, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok(Json(sub))
}

/// POST /api/pdf/forms/submissions/:id/submit
#[utoipa::path(
    post, path = "/api/pdf/forms/submissions/{id}/submit", tag = "Submissions",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Submission finalized", body = FormSubmission),
        (status = 404, body = ApiError), (status = 409, body = ApiError), (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn submit_submission(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> Result<Json<FormSubmission>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let sub = SubmissionRepo::submit(&pool, id, &tenant_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok(Json(sub))
}

/// GET /api/pdf/forms/submissions/:id
#[utoipa::path(
    get, path = "/api/pdf/forms/submissions/{id}", tag = "Submissions",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Submission details", body = FormSubmission),
        (status = 404, body = ApiError), (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_submission(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> Result<Json<FormSubmission>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let sub = SubmissionRepo::find_by_id(&pool, id, &tenant_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?
        .ok_or_else(|| with_request_id(ApiError::not_found("Submission not found"), &ctx))?;

    Ok(Json(sub))
}

/// GET /api/pdf/forms/submissions
#[utoipa::path(
    get, path = "/api/pdf/forms/submissions", tag = "Submissions",
    params(ListSubmissionsParams),
    responses(
        (status = 200, description = "Paginated submission list", body = PaginatedResponse<FormSubmission>),
        (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_submissions(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ListSubmissionsParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 100);
    let q = ListSubmissionsQuery {
        tenant_id,
        template_id: params.template_id,
        status: params.status,
        page: Some(page),
        page_size: Some(page_size),
    };
    let (items, total) = SubmissionRepo::list(&pool, &q)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;
    let resp = PaginatedResponse::new(items, page, page_size, total);
    Ok((StatusCode::OK, Json(resp)))
}

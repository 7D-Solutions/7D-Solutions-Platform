//! HTTP handlers for taxonomy and labels — per spec §4.3.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::{
    models::{CreateCategoryCodeRequest, UpdateCategoryCodeRequest, UpsertLabelRequest},
    repo,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

#[derive(Debug, Deserialize)]
pub struct ListCategoriesQuery {
    pub include_inactive: Option<bool>,
}

// ── Categories ────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/categories", tag = "Taxonomy",
    responses((status = 200, body = Vec<crate::domain::models::ComplaintCategoryCode>)),
    security(("bearer" = [])),
)]
pub async fn list_categories(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<ListCategoriesQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::list_category_codes(&state.pool, &tenant_id, q.include_inactive.unwrap_or(false))
        .await
    {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/customer-complaints/categories", tag = "Taxonomy",
    request_body = CreateCategoryCodeRequest,
    responses(
        (status = 201, body = crate::domain::models::ComplaintCategoryCode),
        (status = 422, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateCategoryCodeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::create_category_code(&state.pool, &tenant_id, &req).await {
        Ok(cat) => (StatusCode::CREATED, Json(cat)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/customer-complaints/categories/{code}", tag = "Taxonomy",
    params(("code" = String, Path, description = "Category code")),
    request_body = UpdateCategoryCodeRequest,
    responses(
        (status = 200, body = crate::domain::models::ComplaintCategoryCode),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(code): Path<String>,
    Json(req): Json<UpdateCategoryCodeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::update_category_code(&state.pool, &tenant_id, &code, &req).await {
        Ok(cat) => Json(cat).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── Status Labels ─────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/status-labels", tag = "Taxonomy",
    responses((status = 200, body = Vec<crate::domain::models::CcStatusLabel>)),
    security(("bearer" = [])),
)]
pub async fn list_status_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::list_status_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/customer-complaints/status-labels/{canonical}", tag = "Taxonomy",
    params(("canonical" = String, Path, description = "Canonical status value")),
    request_body = UpsertLabelRequest,
    responses((status = 200, body = crate::domain::models::CcStatusLabel)),
    security(("bearer" = [])),
)]
pub async fn set_status_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(canonical): Path<String>,
    Json(req): Json<UpsertLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::upsert_status_label(&state.pool, &tenant_id, &canonical, &req).await {
        Ok(label) => Json(label).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── Severity Labels ───────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/severity-labels", tag = "Taxonomy",
    responses((status = 200, body = Vec<crate::domain::models::CcSeverityLabel>)),
    security(("bearer" = [])),
)]
pub async fn list_severity_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::list_severity_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/customer-complaints/severity-labels/{canonical}", tag = "Taxonomy",
    params(("canonical" = String, Path, description = "Canonical severity value")),
    request_body = UpsertLabelRequest,
    responses((status = 200, body = crate::domain::models::CcSeverityLabel)),
    security(("bearer" = [])),
)]
pub async fn set_severity_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(canonical): Path<String>,
    Json(req): Json<UpsertLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::upsert_severity_label(&state.pool, &tenant_id, &canonical, &req).await {
        Ok(label) => Json(label).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ── Source Labels ─────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/customer-complaints/source-labels", tag = "Taxonomy",
    responses((status = 200, body = Vec<crate::domain::models::CcSourceLabel>)),
    security(("bearer" = [])),
)]
pub async fn list_source_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::list_source_labels(&state.pool, &tenant_id).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/customer-complaints/source-labels/{canonical}", tag = "Taxonomy",
    params(("canonical" = String, Path, description = "Canonical source value")),
    request_body = UpsertLabelRequest,
    responses((status = 200, body = crate::domain::models::CcSourceLabel)),
    security(("bearer" = [])),
)]
pub async fn set_source_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(canonical): Path<String>,
    Json(req): Json<UpsertLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::upsert_source_label(&state.pool, &tenant_id, &canonical, &req).await {
        Ok(label) => Json(label).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

//! Form template HTTP handlers.
//!
//! Endpoints:
//!   POST /api/pdf/forms/templates           — create template
//!   GET  /api/pdf/forms/templates           — list templates
//!   GET  /api/pdf/forms/templates/:id       — get template
//!   PUT  /api/pdf/forms/templates/:id       — update template

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::forms::{
    CreateTemplateRequest, FormError, ListTemplatesQuery, TemplateRepo, UpdateTemplateRequest,
};

#[derive(Debug, Deserialize)]
pub struct TenantQuery {
    pub tenant_id: String,
}

fn form_error_response(err: FormError) -> impl IntoResponse {
    match err {
        FormError::TemplateNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Template not found" })),
        ),
        FormError::FieldNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Field not found" })),
        ),
        FormError::DuplicateFieldKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "duplicate_field_key", "message": "Field key already exists on this template" })),
        ),
        FormError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        FormError::Database(e) => {
            tracing::error!(error = %e, "form database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/pdf/forms/templates
pub async fn create_template(
    State(pool): State<PgPool>,
    Json(req): Json<CreateTemplateRequest>,
) -> impl IntoResponse {
    match TemplateRepo::create(&pool, &req).await {
        Ok(tmpl) => (StatusCode::CREATED, Json(json!(tmpl))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

/// GET /api/pdf/forms/templates
pub async fn list_templates(
    State(pool): State<PgPool>,
    Query(q): Query<ListTemplatesQuery>,
) -> impl IntoResponse {
    match TemplateRepo::list(&pool, &q).await {
        Ok(list) => (StatusCode::OK, Json(json!(list))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

/// GET /api/pdf/forms/templates/:id
pub async fn get_template(
    State(pool): State<PgPool>,
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

    match TemplateRepo::find_by_id(&pool, id, &q.tenant_id).await {
        Ok(Some(tmpl)) => (StatusCode::OK, Json(json!(tmpl))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Template not found" })),
        )
            .into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

/// PUT /api/pdf/forms/templates/:id
pub async fn update_template(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
    Json(req): Json<UpdateTemplateRequest>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match TemplateRepo::update(&pool, id, &q.tenant_id, &req).await {
        Ok(tmpl) => (StatusCode::OK, Json(json!(tmpl))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

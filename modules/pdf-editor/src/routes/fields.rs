//! Form field HTTP handlers.
//!
//! Endpoints:
//!   POST /api/pdf/forms/templates/:id/fields          — create field
//!   GET  /api/pdf/forms/templates/:id/fields           — list fields
//!   PUT  /api/pdf/forms/templates/:tid/fields/:fid     — update field
//!   POST /api/pdf/forms/templates/:id/fields/reorder   — reorder fields

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::forms::{
    CreateFieldRequest, FieldRepo, FormError, ReorderFieldsRequest, UpdateFieldRequest,
};

use super::templates::extract_tenant;

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
            tracing::error!(error = %e, "form field database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/pdf/forms/templates/:id/fields
pub async fn create_field(
    State(pool): State<PgPool>,
    Path(template_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateFieldRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match FieldRepo::create(&pool, template_id, &tenant_id, &req).await {
        Ok(field) => (StatusCode::CREATED, Json(json!(field))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

/// GET /api/pdf/forms/templates/:id/fields
pub async fn list_fields(
    State(pool): State<PgPool>,
    Path(template_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match FieldRepo::list(&pool, template_id, &tenant_id).await {
        Ok(fields) => (StatusCode::OK, Json(json!(fields))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

/// PUT /api/pdf/forms/templates/:tid/fields/:fid
pub async fn update_field(
    State(pool): State<PgPool>,
    Path((template_id, field_id)): Path<(Uuid, Uuid)>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<UpdateFieldRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match FieldRepo::update(&pool, field_id, template_id, &tenant_id, &req).await {
        Ok(field) => (StatusCode::OK, Json(json!(field))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

/// POST /api/pdf/forms/templates/:id/fields/reorder
pub async fn reorder_fields(
    State(pool): State<PgPool>,
    Path(template_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<ReorderFieldsRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match FieldRepo::reorder(&pool, template_id, &tenant_id, &req).await {
        Ok(fields) => (StatusCode::OK, Json(json!(fields))).into_response(),
        Err(e) => form_error_response(e).into_response(),
    }
}

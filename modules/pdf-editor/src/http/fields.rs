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
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::forms::{
    CreateFieldRequest, FieldRepo, FormField, ReorderFieldsRequest, UpdateFieldRequest,
};

use super::tenant::{extract_tenant, with_request_id};

/// POST /api/pdf/forms/templates/:id/fields
#[utoipa::path(
    post, path = "/api/pdf/forms/templates/{id}/fields", tag = "Fields",
    params(("id" = Uuid, Path, description = "Template ID")),
    request_body = CreateFieldRequest,
    responses(
        (status = 201, description = "Field created", body = FormField),
        (status = 400, body = ApiError), (status = 404, body = ApiError), (status = 409, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_field(
    State(pool): State<PgPool>,
    Path(template_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateFieldRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let field = FieldRepo::create(&pool, template_id, &tenant_id, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::CREATED, Json(field)))
}

/// GET /api/pdf/forms/templates/:id/fields
#[utoipa::path(
    get, path = "/api/pdf/forms/templates/{id}/fields", tag = "Fields",
    params(("id" = Uuid, Path, description = "Template ID")),
    responses(
        (status = 200, description = "List of fields", body = Vec<FormField>),
        (status = 404, body = ApiError), (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_fields(
    State(pool): State<PgPool>,
    Path(template_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> Result<Json<Vec<FormField>>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let fields = FieldRepo::list(&pool, template_id, &tenant_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok(Json(fields))
}

/// PUT /api/pdf/forms/templates/:tid/fields/:fid
#[utoipa::path(
    put, path = "/api/pdf/forms/templates/{tid}/fields/{fid}", tag = "Fields",
    params(
        ("tid" = Uuid, Path, description = "Template ID"),
        ("fid" = Uuid, Path, description = "Field ID"),
    ),
    request_body = UpdateFieldRequest,
    responses(
        (status = 200, description = "Field updated", body = FormField),
        (status = 404, body = ApiError), (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_field(
    State(pool): State<PgPool>,
    Path((template_id, field_id)): Path<(Uuid, Uuid)>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<UpdateFieldRequest>,
) -> Result<Json<FormField>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let field = FieldRepo::update(&pool, field_id, template_id, &tenant_id, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok(Json(field))
}

/// POST /api/pdf/forms/templates/:id/fields/reorder
#[utoipa::path(
    post, path = "/api/pdf/forms/templates/{id}/fields/reorder", tag = "Fields",
    params(("id" = Uuid, Path, description = "Template ID")),
    request_body = ReorderFieldsRequest,
    responses(
        (status = 200, description = "Reordered fields", body = Vec<FormField>),
        (status = 404, body = ApiError), (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn reorder_fields(
    State(pool): State<PgPool>,
    Path(template_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<ReorderFieldsRequest>,
) -> Result<Json<Vec<FormField>>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let fields = FieldRepo::reorder(&pool, template_id, &tenant_id, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok(Json(fields))
}

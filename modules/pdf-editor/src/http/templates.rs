//! Form template HTTP handlers.
//!
//! Endpoints:
//!   POST /api/pdf/forms/templates           — create template
//!   GET  /api/pdf/forms/templates           — list templates (paginated)
//!   GET  /api/pdf/forms/templates/:id       — get template
//!   PUT  /api/pdf/forms/templates/:id       — update template

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

use crate::domain::forms::{
    CreateTemplateRequest, FormTemplate, ListTemplatesQuery, TemplateRepo, UpdateTemplateRequest,
};

use super::tenant::{extract_tenant, with_request_id};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTemplatesParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// POST /api/pdf/forms/templates
#[utoipa::path(
    post, path = "/api/pdf/forms/templates", tag = "Templates",
    request_body = CreateTemplateRequest,
    responses(
        (status = 201, description = "Template created", body = FormTemplate),
        (status = 400, body = ApiError), (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_template(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateTemplateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;
    let tmpl = TemplateRepo::create(&pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;
    Ok((StatusCode::CREATED, Json(tmpl)))
}

/// GET /api/pdf/forms/templates
#[utoipa::path(
    get, path = "/api/pdf/forms/templates", tag = "Templates",
    params(ListTemplatesParams),
    responses(
        (status = 200, description = "Paginated template list", body = PaginatedResponse<FormTemplate>),
        (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_templates(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ListTemplatesParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 100);
    let q = ListTemplatesQuery {
        tenant_id,
        page: Some(page),
        page_size: Some(page_size),
    };
    let (items, total) = TemplateRepo::list(&pool, &q)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;
    let resp = PaginatedResponse::new(items, page, page_size, total);
    Ok((StatusCode::OK, Json(resp)))
}

/// GET /api/pdf/forms/templates/:id
#[utoipa::path(
    get, path = "/api/pdf/forms/templates/{id}", tag = "Templates",
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Template details", body = FormTemplate),
        (status = 404, body = ApiError), (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_template(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> Result<Json<FormTemplate>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let tmpl = TemplateRepo::find_by_id(&pool, id, &tenant_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?
        .ok_or_else(|| with_request_id(ApiError::not_found("Template not found"), &ctx))?;

    Ok(Json(tmpl))
}

/// PUT /api/pdf/forms/templates/:id
#[utoipa::path(
    put, path = "/api/pdf/forms/templates/{id}", tag = "Templates",
    params(("id" = Uuid, Path)),
    request_body = UpdateTemplateRequest,
    responses(
        (status = 200, description = "Template updated", body = FormTemplate),
        (status = 404, body = ApiError), (status = 400, body = ApiError), (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_template(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<UpdateTemplateRequest>,
) -> Result<Json<FormTemplate>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let tmpl = TemplateRepo::update(&pool, id, &tenant_id, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok(Json(tmpl))
}

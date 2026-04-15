//! Workflow definition HTTP handlers.

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
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::definitions::{
    CreateDefinitionRequest, DefinitionRepo, ListDefinitionsQuery, WorkflowDefinition,
};
use crate::AppState;

use super::tenant::with_request_id;
use platform_sdk::extract_tenant;

#[utoipa::path(
    post, path = "/api/workflow/definitions", tag = "Definitions",
    request_body = CreateDefinitionRequest,
    responses(
        (status = 201, description = "Definition created", body = WorkflowDefinition),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 409, description = "Duplicate", body = ApiError),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn create_definition(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateDefinitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match DefinitionRepo::create(&state.pool, &req).await {
        Ok(def) => (StatusCode::CREATED, Json(def)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListDefinitionsParams {
    pub active_only: Option<bool>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get, path = "/api/workflow/definitions", tag = "Definitions",
    params(ListDefinitionsParams),
    responses(
        (status = 200, description = "Paginated definitions", body = PaginatedResponse<WorkflowDefinition>),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn list_definitions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ListDefinitionsParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 200);
    let q = ListDefinitionsQuery {
        tenant_id,
        active_only: params.active_only,
        limit: Some(page_size),
        offset: Some((page - 1) * page_size),
    };
    let total = match DefinitionRepo::count(&state.pool, &q).await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(e), &ctx).into_response(),
    };
    match DefinitionRepo::list(&state.pool, &q).await {
        Ok(defs) => Json(PaginatedResponse::new(defs, page, page_size, total)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/workflow/definitions/{def_id}", tag = "Definitions",
    params(("def_id" = Uuid, Path, description = "Definition ID")),
    responses(
        (status = 200, description = "Definition found", body = WorkflowDefinition),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn get_definition(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(def_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match DefinitionRepo::get(&state.pool, &tenant_id, def_id).await {
        Ok(def) => Json(def).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

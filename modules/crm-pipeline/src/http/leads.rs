//! HTTP handlers for leads — 10 endpoints per spec §4.1.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::leads::{
    service, ConvertLeadRequest, CreateLeadRequest, DisqualifyLeadRequest, ListLeadsQuery,
    UpdateLeadRequest,
};
use crate::http::tenant::{correlation_from_headers, with_request_id};
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads", tag = "Leads",
    request_body = CreateLeadRequest,
    responses((status = 201, description = "Lead created", body = crate::domain::leads::Lead), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_lead(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateLeadRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::create_lead(&state.pool, &tenant_id, &req, actor).await {
        Ok(lead) => (StatusCode::CREATED, Json(lead)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/leads/{id}", tag = "Leads",
    params(("id" = Uuid, Path, description = "Lead ID")),
    responses((status = 200, body = crate::domain::leads::Lead), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_lead(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::get_lead(&state.pool, &tenant_id, id).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/leads", tag = "Leads",
    responses((status = 200, body = PaginatedResponse<crate::domain::leads::Lead>)),
    security(("bearer" = [])),
)]
pub async fn list_leads(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListLeadsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::list_leads(&state.pool, &tenant_id, &query).await {
        Ok(leads) => {
            let total = leads.len() as i64;
            Json(PaginatedResponse::new(leads, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/crm-pipeline/leads/{id}", tag = "Leads",
    request_body = UpdateLeadRequest,
    responses((status = 200, body = crate::domain::leads::Lead), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_lead(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLeadRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::update_lead(&state.pool, &tenant_id, id, &req).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads/{id}/contact", tag = "Leads",
    responses((status = 200, body = crate::domain::leads::Lead)),
    security(("bearer" = [])),
)]
pub async fn mark_contacted(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::mark_contacted(&state.pool, &tenant_id, id, actor).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads/{id}/qualify", tag = "Leads",
    responses((status = 200, body = crate::domain::leads::Lead)),
    security(("bearer" = [])),
)]
pub async fn mark_qualifying(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::mark_qualifying(&state.pool, &tenant_id, id, actor).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads/{id}/mark-qualified", tag = "Leads",
    responses((status = 200, body = crate::domain::leads::Lead)),
    security(("bearer" = [])),
)]
pub async fn mark_qualified(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::mark_qualified(&state.pool, &tenant_id, id, actor).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads/{id}/convert", tag = "Leads",
    request_body = ConvertLeadRequest,
    responses((status = 200, body = crate::domain::leads::ConvertLeadResponse), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn convert_lead(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ConvertLeadRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::convert_lead(&state.pool, &tenant_id, id, &req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads/{id}/disqualify", tag = "Leads",
    request_body = DisqualifyLeadRequest,
    responses((status = 200, body = crate::domain::leads::Lead), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn disqualify_lead(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<DisqualifyLeadRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::disqualify_lead(&state.pool, &tenant_id, id, &req).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/leads/{id}/mark-dead", tag = "Leads",
    responses((status = 200, body = crate::domain::leads::Lead)),
    security(("bearer" = [])),
)]
pub async fn mark_dead(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::mark_dead(&state.pool, &tenant_id, id).await {
        Ok(lead) => Json(lead).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

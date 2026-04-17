//! HTTP handlers for opportunities — 9 endpoints per spec §4.2.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::opportunities::{
    service, AdvanceStageRequest, CloseLostRequest, CloseWonRequest, CreateOpportunityRequest,
    ListOpportunitiesQuery, UpdateOpportunityRequest,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    post, path = "/api/crm-pipeline/opportunities", tag = "Opportunities",
    request_body = CreateOpportunityRequest,
    responses((status = 201, body = crate::domain::opportunities::Opportunity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_opportunity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateOpportunityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::create_opportunity(&state.pool, &tenant_id, &req, actor).await {
        Ok(opp) => (StatusCode::CREATED, Json(opp)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/opportunities/{id}", tag = "Opportunities",
    params(("id" = Uuid, Path, description = "Opportunity ID")),
    responses((status = 200, body = crate::domain::opportunities::OpportunityDetail), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_opportunity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::get_opportunity_detail(&state.pool, &tenant_id, id).await {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/opportunities", tag = "Opportunities",
    responses((status = 200, body = PaginatedResponse<crate::domain::opportunities::Opportunity>)),
    security(("bearer" = [])),
)]
pub async fn list_opportunities(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListOpportunitiesQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::list_opportunities(&state.pool, &tenant_id, &query).await {
        Ok(opps) => {
            let total = opps.len() as i64;
            Json(PaginatedResponse::new(opps, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/crm-pipeline/opportunities/{id}", tag = "Opportunities",
    request_body = UpdateOpportunityRequest,
    responses((status = 200, body = crate::domain::opportunities::Opportunity)),
    security(("bearer" = [])),
)]
pub async fn update_opportunity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateOpportunityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::update_opportunity(&state.pool, &tenant_id, id, &req).await {
        Ok(opp) => Json(opp).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/opportunities/{id}/advance-stage", tag = "Opportunities",
    request_body = AdvanceStageRequest,
    responses((status = 200, body = crate::domain::opportunities::Opportunity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn advance_stage(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AdvanceStageRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::advance_stage(&state.pool, &tenant_id, id, &req, actor).await {
        Ok(opp) => Json(opp).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/opportunities/{id}/close-won", tag = "Opportunities",
    request_body = CloseWonRequest,
    responses((status = 200, body = crate::domain::opportunities::Opportunity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn close_won(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CloseWonRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::close_won(&state.pool, &tenant_id, id, &req, actor).await {
        Ok(opp) => Json(opp).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/opportunities/{id}/close-lost", tag = "Opportunities",
    request_body = CloseLostRequest,
    responses((status = 200, body = crate::domain::opportunities::Opportunity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn close_lost(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CloseLostRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match service::close_lost(&state.pool, &tenant_id, id, &req, actor).await {
        Ok(opp) => Json(opp).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/opportunities/{id}/stage-history", tag = "Opportunities",
    params(("id" = Uuid, Path, description = "Opportunity ID")),
    responses((status = 200, body = Vec<crate::domain::opportunities::OpportunityStageHistory>)),
    security(("bearer" = [])),
)]
pub async fn stage_history(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::list_stage_history(&state.pool, &tenant_id, id).await {
        Ok(history) => Json(history).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PipelineSummaryQuery {
    pub owner_id: Option<String>,
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/pipeline/summary", tag = "Opportunities",
    responses((status = 200, body = Vec<crate::domain::opportunities::PipelineSummaryItem>)),
    security(("bearer" = [])),
)]
pub async fn pipeline_summary(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<PipelineSummaryQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::pipeline_summary(&state.pool, &tenant_id, query.owner_id.as_deref()).await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

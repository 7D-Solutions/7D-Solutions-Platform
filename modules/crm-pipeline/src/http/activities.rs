//! HTTP handlers for activities and activity types — 8 endpoints per spec §4.4.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::activities::{
    service as act_service, CreateActivityRequest, ListActivitiesQuery, UpdateActivityRequest,
};
use crate::domain::activity_types::{
    repo as atype_repo, CreateActivityTypeRequest, UpdateActivityTypeRequest,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    post, path = "/api/crm-pipeline/activities", tag = "Activities",
    request_body = CreateActivityRequest,
    responses((status = 201, body = crate::domain::activities::Activity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn log_activity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateActivityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match act_service::log_activity(&state.pool, &tenant_id, &req, actor).await {
        Ok(act) => (StatusCode::CREATED, Json(act)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/activities/{id}", tag = "Activities",
    params(("id" = Uuid, Path, description = "Activity ID")),
    responses((status = 200, body = crate::domain::activities::Activity), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_activity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match act_service::get_activity(&state.pool, &tenant_id, id).await {
        Ok(act) => Json(act).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/activities", tag = "Activities",
    responses((status = 200, body = PaginatedResponse<crate::domain::activities::Activity>)),
    security(("bearer" = [])),
)]
pub async fn list_activities(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListActivitiesQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match act_service::list_activities(&state.pool, &tenant_id, &query).await {
        Ok(acts) => {
            let total = acts.len() as i64;
            Json(PaginatedResponse::new(acts, 1, total, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/activities/{id}/complete", tag = "Activities",
    responses((status = 200, body = crate::domain::activities::Activity), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn complete_activity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match act_service::complete_activity(&state.pool, &tenant_id, id, actor).await {
        Ok(act) => Json(act).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/crm-pipeline/activities/{id}", tag = "Activities",
    request_body = UpdateActivityRequest,
    responses((status = 200, body = crate::domain::activities::Activity)),
    security(("bearer" = [])),
)]
pub async fn update_activity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateActivityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match act_service::update_activity(&state.pool, &tenant_id, id, &req).await {
        Ok(act) => Json(act).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/crm-pipeline/activity-types", tag = "Activities",
    responses((status = 200, body = Vec<crate::domain::activity_types::ActivityType>)),
    security(("bearer" = [])),
)]
pub async fn list_activity_types(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match atype_repo::list_activity_types(&state.pool, &tenant_id).await {
        Ok(types) => Json(types).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post, path = "/api/crm-pipeline/activity-types", tag = "Activities",
    request_body = CreateActivityTypeRequest,
    responses((status = 201, body = crate::domain::activity_types::ActivityType), (status = 422, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_activity_type(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateActivityTypeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match atype_repo::create_activity_type(&state.pool, &tenant_id, &req, &actor).await {
        Ok(at) => (StatusCode::CREATED, Json(at)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/crm-pipeline/activity-types/{code}", tag = "Activities",
    request_body = UpdateActivityTypeRequest,
    responses((status = 200, body = crate::domain::activity_types::ActivityType)),
    security(("bearer" = [])),
)]
pub async fn update_activity_type(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(code): Path<String>,
    Json(req): Json<UpdateActivityTypeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match atype_repo::update_activity_type(&state.pool, &tenant_id, &code, &req, &actor).await {
        Ok(at) => Json(at).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

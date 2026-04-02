use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::pagination::PaginationQuery;
use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::routings::{
        AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo, RoutingStep, RoutingTemplate,
        UpdateRoutingRequest,
    },
    AppState,
};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ItemDateQuery {
    pub item_id: Uuid,
    pub effective_date: NaiveDate,
}

/// POST /api/production/routings
#[utoipa::path(
    post,
    path = "/api/production/routings",
    tag = "Routings",
    request_body = CreateRoutingRequest,
    responses(
        (status = 201, description = "Routing created", body = RoutingTemplate),
        (status = 409, description = "Duplicate revision", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_routing(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateRoutingRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::create(&state.pool, &req, &corr, None).await {
        Ok(rt) => (StatusCode::CREATED, Json(rt)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/routings/:id
#[utoipa::path(
    get,
    path = "/api/production/routings/{id}",
    tag = "Routings",
    params(("id" = Uuid, Path, description = "Routing template ID")),
    responses(
        (status = 200, description = "Routing details", body = RoutingTemplate),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_routing(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match RoutingRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(rt)) => (StatusCode::OK, Json(rt)).into_response(),
        Ok(None) => {
            with_request_id(ApiError::not_found("Routing not found"), &tracing_ctx)
                .into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/routings
#[utoipa::path(
    get,
    path = "/api/production/routings",
    tag = "Routings",
    params(PaginationQuery),
    responses(
        (status = 200, description = "Paginated routing list", body = PaginatedResponse<RoutingTemplate>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_routings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(pq): Query<PaginationQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = pq.page.max(1);
    let page_size = pq.page_size.clamp(1, 200);
    match RoutingRepo::list(&state.pool, &tenant_id, page, page_size).await {
        Ok((items, total)) => {
            let resp = PaginatedResponse::new(items, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/routings/by-item?item_id=...&effective_date=...
#[utoipa::path(
    get,
    path = "/api/production/routings/by-item",
    tag = "Routings",
    params(ItemDateQuery),
    responses(
        (status = 200, description = "Routings matching item and date", body = PaginatedResponse<RoutingTemplate>),
    ),
    security(("bearer" = [])),
)]
pub async fn find_routings_by_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ItemDateQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match RoutingRepo::find_by_item_and_date(
        &state.pool,
        &tenant_id,
        params.item_id,
        params.effective_date,
    )
    .await
    {
        Ok(rts) => {
            let total = rts.len() as i64;
            let resp = PaginatedResponse::new(rts, 1, total, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// PUT /api/production/routings/:id
#[utoipa::path(
    put,
    path = "/api/production/routings/{id}",
    tag = "Routings",
    params(("id" = Uuid, Path, description = "Routing template ID")),
    request_body = UpdateRoutingRequest,
    responses(
        (status = 200, description = "Routing updated", body = RoutingTemplate),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Released immutable", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_routing(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateRoutingRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::update(&state.pool, id, &req, &corr, None).await {
        Ok(rt) => (StatusCode::OK, Json(rt)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/routings/:id/release
#[utoipa::path(
    post,
    path = "/api/production/routings/{id}/release",
    tag = "Routings",
    params(("id" = Uuid, Path, description = "Routing template ID")),
    responses(
        (status = 200, description = "Routing released", body = RoutingTemplate),
        (status = 404, description = "Not found", body = ApiError),
        (status = 409, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn release_routing(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::release(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(rt) => (StatusCode::OK, Json(rt)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// POST /api/production/routings/:id/steps
#[utoipa::path(
    post,
    path = "/api/production/routings/{id}/steps",
    tag = "Routings",
    params(("id" = Uuid, Path, description = "Routing template ID")),
    request_body = AddRoutingStepRequest,
    responses(
        (status = 201, description = "Step added", body = RoutingStep),
        (status = 404, description = "Routing not found", body = ApiError),
        (status = 409, description = "Duplicate sequence or released", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_routing_step(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<AddRoutingStepRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::add_step(&state.pool, id, &req, &corr, None).await {
        Ok(step) => (StatusCode::CREATED, Json(step)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/production/routings/:id/steps
#[utoipa::path(
    get,
    path = "/api/production/routings/{id}/steps",
    tag = "Routings",
    params(("id" = Uuid, Path, description = "Routing template ID")),
    responses(
        (status = 200, description = "Routing steps", body = PaginatedResponse<RoutingStep>),
        (status = 404, description = "Routing not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_routing_steps(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match RoutingRepo::list_steps(&state.pool, id, &tenant_id).await {
        Ok(steps) => {
            let total = steps.len() as i64;
            let resp = PaginatedResponse::new(steps, 1, total, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

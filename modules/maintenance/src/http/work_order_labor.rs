use axum::{extract::{Path, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;
use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::work_orders::{AddLaborRequest, WoLabor, WoLaborRepo};
use crate::AppState;

#[utoipa::path(
    post, path = "/api/maintenance/work-orders/{wo_id}/labor", tag = "Work Order Labor",
    params(("wo_id" = Uuid, Path, description = "Work order ID")),
    request_body = AddLaborRequest,
    responses(
        (status = 201, description = "Labor entry added", body = WoLabor),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn add_labor(State(state): State<Arc<AppState>>, Path(wo_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<AddLaborRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match WoLaborRepo::add(&state.pool, wo_id, &req).await { Ok(l) => (StatusCode::CREATED, Json(l)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/work-orders/{wo_id}/labor", tag = "Work Order Labor",
    params(("wo_id" = Uuid, Path, description = "Work order ID")),
    responses(
        (status = 200, description = "Labor entries for work order", body = PaginatedResponse<WoLabor>),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_labor(State(state): State<Arc<AppState>>, Path(wo_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match WoLaborRepo::list(&state.pool, wo_id, &tenant_id).await { Ok(v) => { let t = v.len() as i64; (StatusCode::OK, Json(PaginatedResponse::new(v, 1, t.max(1), t))).into_response() } Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    delete, path = "/api/maintenance/work-orders/{wo_id}/labor/{labor_id}", tag = "Work Order Labor",
    params(
        ("wo_id" = Uuid, Path, description = "Work order ID"),
        ("labor_id" = Uuid, Path, description = "Labor entry ID"),
    ),
    responses(
        (status = 204, description = "Labor entry removed"),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn remove_labor(State(state): State<Arc<AppState>>, Path((wo_id, labor_id)): Path<(Uuid, Uuid)>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match WoLaborRepo::remove(&state.pool, wo_id, labor_id, &tenant_id).await { Ok(()) => StatusCode::NO_CONTENT.into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

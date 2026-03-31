use axum::{extract::{Path, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;
use super::tenant::{extract_tenant, with_request_id};
use crate::domain::work_orders::{AddPartRequest, WoPartsRepo};
use crate::AppState;

pub async fn add_part(State(state): State<Arc<AppState>>, Path(wo_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<AddPartRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match WoPartsRepo::add(&state.pool, wo_id, &req).await { Ok(p) => (StatusCode::CREATED, Json(p)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}
pub async fn list_parts(State(state): State<Arc<AppState>>, Path(wo_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match WoPartsRepo::list(&state.pool, wo_id, &tenant_id).await { Ok(v) => { let t = v.len() as i64; (StatusCode::OK, Json(PaginatedResponse::new(v, 1, t.max(1), t))).into_response() } Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}
pub async fn remove_part(State(state): State<Arc<AppState>>, Path((wo_id, part_id)): Path<(Uuid, Uuid)>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match WoPartsRepo::remove(&state.pool, wo_id, part_id, &tenant_id).await { Ok(()) => StatusCode::NO_CONTENT.into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

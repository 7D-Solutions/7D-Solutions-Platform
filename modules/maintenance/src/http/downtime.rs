use axum::{extract::{Path, Query, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
use super::tenant::{extract_tenant, with_request_id};
use crate::domain::downtime::{CreateDowntimeRequest, DowntimeError, DowntimeRepo, ListDowntimeQuery};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListDowntimeParams { pub asset_id: Option<Uuid>, pub from: Option<DateTime<Utc>>, pub to: Option<DateTime<Utc>>, pub page: Option<i64>, pub page_size: Option<i64> }

pub async fn create_downtime(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<CreateDowntimeRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match DowntimeRepo::create(&state.pool, &req).await { Ok(ev) => (StatusCode::CREATED, Json(ev)).into_response(), Err(DowntimeError::IdempotentDuplicate(ev)) => (StatusCode::OK, Json(*ev)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}
pub async fn list_downtime(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, Query(params): Query<ListDowntimeParams>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    let page = params.page.unwrap_or(1).max(1); let page_size = params.page_size.unwrap_or(50).clamp(1, 200); let offset = (page - 1) * page_size;
    let q = ListDowntimeQuery { tenant_id, asset_id: params.asset_id, from: params.from, to: params.to, limit: Some(page_size), offset: Some(offset) };
    let total = match DowntimeRepo::count(&state.pool, &q).await { Ok(t) => t, Err(e) => { let a = ApiError::from(e); return with_request_id(a, &tracing_ctx).into_response(); } };
    match DowntimeRepo::list(&state.pool, &q).await { Ok(v) => (StatusCode::OK, Json(PaginatedResponse::new(v, page, page_size, total))).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}
pub async fn get_downtime(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match DowntimeRepo::find_by_id(&state.pool, id, &tenant_id).await { Ok(Some(ev)) => (StatusCode::OK, Json(ev)).into_response(), Ok(None) => with_request_id(ApiError::not_found("Downtime event not found"), &tracing_ctx).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}
pub async fn list_asset_downtime(State(state): State<Arc<AppState>>, Path(asset_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match DowntimeRepo::list_for_asset(&state.pool, asset_id, &tenant_id).await { Ok(v) => { let t = v.len() as i64; (StatusCode::OK, Json(PaginatedResponse::new(v, 1, t.max(1), t))).into_response() } Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

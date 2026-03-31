use axum::{extract::{Path, Query, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
use super::tenant::{extract_tenant, with_request_id};
use crate::domain::work_orders::{CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WorkOrderRepo};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListWorkOrdersParams { pub asset_id: Option<Uuid>, pub status: Option<String>, pub page: Option<i64>, pub page_size: Option<i64> }

pub async fn create_work_order(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<CreateWorkOrderRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match WorkOrderRepo::create(&state.pool, &req).await {
        Ok(wo) => (StatusCode::CREATED, Json(wo)).into_response(),
        Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() }
    }
}

pub async fn list_work_orders(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, Query(params): Query<ListWorkOrdersParams>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;
    let q = ListWorkOrdersQuery { tenant_id, asset_id: params.asset_id, status: params.status, limit: Some(page_size), offset: Some(offset) };
    let total = match WorkOrderRepo::count(&state.pool, &q).await { Ok(t) => t, Err(e) => { let a = ApiError::from(e); return with_request_id(a, &tracing_ctx).into_response(); } };
    match WorkOrderRepo::list(&state.pool, &q).await {
        Ok(orders) => (StatusCode::OK, Json(PaginatedResponse::new(orders, page, page_size, total))).into_response(),
        Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() }
    }
}

pub async fn get_work_order(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match WorkOrderRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(wo)) => (StatusCode::OK, Json(wo)).into_response(),
        Ok(None) => with_request_id(ApiError::not_found("Work order not found"), &tracing_ctx).into_response(),
        Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() }
    }
}

pub async fn transition_work_order(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<TransitionRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match WorkOrderRepo::transition(&state.pool, id, &req).await {
        Ok(wo) => (StatusCode::OK, Json(wo)).into_response(),
        Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() }
    }
}

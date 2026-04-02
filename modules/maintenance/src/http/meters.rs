use axum::{extract::{Path, Query, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::meters::{CreateMeterTypeRequest, ListReadingsQuery, MeterReading, MeterReadingRepo, MeterType, MeterTypeRepo, RecordReadingRequest};
use crate::AppState;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListReadingsParams {
    pub meter_type_id: Option<Uuid>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    post, path = "/api/maintenance/meter-types", tag = "Meters",
    request_body = CreateMeterTypeRequest,
    responses(
        (status = 201, description = "Meter type created", body = MeterType),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_meter_type(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<CreateMeterTypeRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match MeterTypeRepo::create(&state.pool, &req).await { Ok(mt) => (StatusCode::CREATED, Json(mt)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/meter-types", tag = "Meters",
    responses(
        (status = 200, description = "List of meter types", body = PaginatedResponse<MeterType>),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_meter_types(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match MeterTypeRepo::list(&state.pool, &tenant_id).await { Ok(v) => { let t = v.len() as i64; (StatusCode::OK, Json(PaginatedResponse::new(v, 1, t.max(1), t))).into_response() } Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    post, path = "/api/maintenance/assets/{asset_id}/readings", tag = "Meters",
    params(("asset_id" = Uuid, Path, description = "Asset ID")),
    request_body = RecordReadingRequest,
    responses(
        (status = 201, description = "Meter reading recorded", body = MeterReading),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn record_reading(State(state): State<Arc<AppState>>, Path(asset_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<RecordReadingRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match MeterReadingRepo::record(&state.pool, asset_id, &req).await { Ok(r) => (StatusCode::CREATED, Json(r)).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

#[utoipa::path(
    get, path = "/api/maintenance/assets/{asset_id}/readings", tag = "Meters",
    params(("asset_id" = Uuid, Path, description = "Asset ID"), ListReadingsParams),
    responses(
        (status = 200, description = "Paginated meter readings", body = PaginatedResponse<MeterReading>),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_readings(State(state): State<Arc<AppState>>, Path(asset_id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, Query(params): Query<ListReadingsParams>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    let page = params.page.unwrap_or(1).max(1); let page_size = params.page_size.unwrap_or(50).clamp(1, 200); let offset = (page - 1) * page_size;
    let q = ListReadingsQuery { meter_type_id: params.meter_type_id, limit: Some(page_size), offset: Some(offset) };
    let total = match MeterReadingRepo::count(&state.pool, &tenant_id, asset_id, &q).await { Ok(t) => t, Err(e) => { let a = ApiError::from(e); return with_request_id(a, &tracing_ctx).into_response(); } };
    match MeterReadingRepo::list(&state.pool, &tenant_id, asset_id, &q).await { Ok(v) => (StatusCode::OK, Json(PaginatedResponse::new(v, page, page_size, total))).into_response(), Err(e) => { let a = ApiError::from(e); with_request_id(a, &tracing_ctx).into_response() } }
}

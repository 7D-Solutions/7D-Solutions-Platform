//! Maintainable asset HTTP handlers.

use axum::{extract::{Path, Query, State}, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::assets::{Asset, AssetError, AssetRepo, CreateAssetRequest, ListAssetsQuery, UpdateAssetRequest};
use crate::AppState;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListAssetsParams {
    pub asset_type: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    post, path = "/api/maintenance/assets", tag = "Assets",
    request_body = CreateAssetRequest,
    responses(
        (status = 201, description = "Asset created", body = Asset),
        (status = 200, description = "Idempotent duplicate", body = Asset),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_asset(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(mut req): Json<CreateAssetRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    req.tenant_id = tenant_id;
    match AssetRepo::create(&state.pool, &req).await {
        Ok(asset) => (StatusCode::CREATED, Json(asset)).into_response(),
        Err(AssetError::IdempotentDuplicate(asset)) => (StatusCode::OK, Json(*asset)).into_response(),
        Err(e) => { let api_err: ApiError = e.into(); with_request_id(api_err, &tracing_ctx).into_response() }
    }
}

#[utoipa::path(
    get, path = "/api/maintenance/assets", tag = "Assets",
    params(ListAssetsParams),
    responses(
        (status = 200, description = "Paginated list of assets", body = PaginatedResponse<Asset>),
        (status = 400, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_assets(State(state): State<Arc<AppState>>, claims: Option<Extension<VerifiedClaims>>, Query(params): Query<ListAssetsParams>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;
    let q = ListAssetsQuery { tenant_id, asset_type: params.asset_type, status: params.status, limit: Some(page_size), offset: Some(offset) };
    match AssetRepo::list(&state.pool, &q).await {
        Ok(resp) => (StatusCode::OK, Json(PaginatedResponse::new(resp.items, page, page_size, resp.total))).into_response(),
        Err(e) => { let api_err: ApiError = e.into(); with_request_id(api_err, &tracing_ctx).into_response() }
    }
}

#[utoipa::path(
    get, path = "/api/maintenance/assets/{asset_id}", tag = "Assets",
    params(("asset_id" = Uuid, Path, description = "Asset ID")),
    responses(
        (status = 200, description = "Asset details", body = Asset),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_asset(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match AssetRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(asset)) => (StatusCode::OK, Json(asset)).into_response(),
        Ok(None) => with_request_id(ApiError::not_found("Asset not found"), &tracing_ctx).into_response(),
        Err(e) => { let api_err: ApiError = e.into(); with_request_id(api_err, &tracing_ctx).into_response() }
    }
}

#[utoipa::path(
    patch, path = "/api/maintenance/assets/{asset_id}", tag = "Assets",
    params(("asset_id" = Uuid, Path, description = "Asset ID")),
    request_body = UpdateAssetRequest,
    responses(
        (status = 200, description = "Asset updated", body = Asset),
        (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_asset(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, claims: Option<Extension<VerifiedClaims>>, tracing_ctx: Option<Extension<TracingContext>>, Json(req): Json<UpdateAssetRequest>) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { Ok(t) => t, Err(e) => return with_request_id(e, &tracing_ctx).into_response() };
    match AssetRepo::update(&state.pool, id, &tenant_id, &req).await {
        Ok(asset) => (StatusCode::OK, Json(asset)).into_response(),
        Err(e) => { let api_err: ApiError = e.into(); with_request_id(api_err, &tracing_ctx).into_response() }
    }
}

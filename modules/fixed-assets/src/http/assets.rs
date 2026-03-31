//! HTTP handlers for Fixed Assets CRUD: categories and assets.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::assets::{
    AssetRepo, CategoryRepo, CreateAssetRequest, CreateCategoryRequest, UpdateAssetRequest,
    UpdateCategoryRequest,
};
use crate::AppState;

use super::helpers::tenant::{extract_tenant, with_request_id};

// ============================================================================
// Category endpoints
// ============================================================================

#[utoipa::path(
    post, path = "/api/fixed-assets/categories", tag = "Categories",
    request_body = CreateCategoryRequest,
    responses((status = 201, description = "Category created"), (status = 401, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateCategoryRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match CategoryRepo::create(&state.pool, &req).await {
        Ok(cat) => (StatusCode::CREATED, Json(cat)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/fixed-assets/categories/{id}", tag = "Categories",
    params(("id" = Uuid, Path, description = "Category ID")),
    request_body = UpdateCategoryRequest,
    responses((status = 200, description = "Category updated"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateCategoryRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match CategoryRepo::update(&state.pool, id, &req).await {
        Ok(cat) => Json(cat).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/fixed-assets/categories/{id}", tag = "Categories",
    params(("id" = Uuid, Path, description = "Category ID")),
    responses((status = 200, description = "Category deactivated"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn deactivate_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match CategoryRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(cat) => Json(cat).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/fixed-assets/categories/{id}", tag = "Categories",
    params(("id" = Uuid, Path, description = "Category ID")),
    responses((status = 200, description = "Category details"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match CategoryRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(cat)) => Json(cat).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Category {} not found", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/fixed-assets/categories", tag = "Categories",
    responses((status = 200, description = "Category list", body = PaginatedResponse<crate::domain::assets::Category>)),
    security(("bearer" = [])),
)]
pub async fn list_categories(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match CategoryRepo::list(&state.pool, &tenant_id).await {
        Ok(cats) => {
            let total = cats.len() as i64;
            let resp = PaginatedResponse::new(cats, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Asset endpoints
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    pub status: Option<String>,
}

#[utoipa::path(
    post, path = "/api/fixed-assets/assets", tag = "Assets",
    request_body = CreateAssetRequest,
    responses((status = 201, description = "Asset created"), (status = 401, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn create_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateAssetRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match AssetRepo::create(&state.pool, &req).await {
        Ok(asset) => (StatusCode::CREATED, Json(asset)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/fixed-assets/assets/{id}", tag = "Assets",
    params(("id" = Uuid, Path, description = "Asset ID")),
    request_body = UpdateAssetRequest,
    responses((status = 200, description = "Asset updated"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn update_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateAssetRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match AssetRepo::update(&state.pool, id, &req).await {
        Ok(asset) => Json(asset).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    delete, path = "/api/fixed-assets/assets/{id}", tag = "Assets",
    params(("id" = Uuid, Path, description = "Asset ID")),
    responses((status = 200, description = "Asset deactivated"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn deactivate_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match AssetRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(asset) => Json(asset).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/fixed-assets/assets/{id}", tag = "Assets",
    params(("id" = Uuid, Path, description = "Asset ID")),
    responses((status = 200, description = "Asset details"), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match AssetRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(asset)) => Json(asset).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Asset {} not found", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/fixed-assets/assets", tag = "Assets",
    responses((status = 200, description = "Asset list", body = PaginatedResponse<crate::domain::assets::Asset>)),
    security(("bearer" = [])),
)]
pub async fn list_assets(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListAssetsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match AssetRepo::list(&state.pool, &tenant_id, query.status.as_deref()).await {
        Ok(assets) => {
            let total = assets.len() as i64;
            let resp = PaginatedResponse::new(assets, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

//! HTTP handlers for Fixed Assets CRUD: categories and assets.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::assets::{
    AssetError, AssetRepo, CategoryRepo, CreateAssetRequest, CreateCategoryRequest,
    UpdateAssetRequest, UpdateCategoryRequest,
};
use crate::AppState;

use super::helpers::tenant::extract_tenant;

// ============================================================================
// Error mapping
// ============================================================================

fn map_error(e: AssetError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, msg) = match &e {
        AssetError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        AssetError::CategoryNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
        AssetError::Validation(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        AssetError::DuplicateTag(_, _) => (StatusCode::CONFLICT, e.to_string()),
        AssetError::DuplicateCategoryCode(_, _) => (StatusCode::CONFLICT, e.to_string()),
        AssetError::InvalidTransition(_) => (StatusCode::CONFLICT, e.to_string()),
        AssetError::Database(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error".to_string(),
        ),
    };
    (status, Json(serde_json::json!({ "error": msg })))
}

fn map_internal_error<E: std::fmt::Display>(e: E) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!(error = %e, "Internal error during serialization");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "Internal error" })),
    )
}

// ============================================================================
// Category endpoints
// ============================================================================

pub async fn create_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateCategoryRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let cat = CategoryRepo::create(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(cat).map_err(map_internal_error)?),
    ))
}

pub async fn update_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateCategoryRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let cat = CategoryRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(cat).map_err(map_internal_error)?))
}

pub async fn deactivate_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let cat = CategoryRepo::deactivate(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(cat).map_err(map_internal_error)?))
}

pub async fn get_category(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let cat = CategoryRepo::find_by_id(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(AssetError::NotFound))?;
    Ok(Json(serde_json::to_value(cat).map_err(map_internal_error)?))
}

pub async fn list_categories(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let cats = CategoryRepo::list(&state.pool, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(cats).map_err(map_internal_error)?))
}

// ============================================================================
// Asset endpoints
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    pub status: Option<String>,
}

pub async fn create_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateAssetRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let asset = AssetRepo::create(&state.pool, &req).await.map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(asset).map_err(map_internal_error)?),
    ))
}

pub async fn update_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateAssetRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let asset = AssetRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(asset).map_err(map_internal_error)?))
}

pub async fn deactivate_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let asset = AssetRepo::deactivate(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(asset).map_err(map_internal_error)?))
}

pub async fn get_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let asset = AssetRepo::find_by_id(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(AssetError::NotFound))?;
    Ok(Json(serde_json::to_value(asset).map_err(map_internal_error)?))
}

pub async fn list_assets(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListAssetsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let assets = AssetRepo::list(&state.pool, &tenant_id, query.status.as_deref())
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(assets).map_err(map_internal_error)?))
}

//! HTTP handlers for Fixed Assets CRUD: categories and assets.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::assets::{
    AssetError, AssetRepo, CategoryRepo, CreateAssetRequest, CreateCategoryRequest,
    UpdateAssetRequest, UpdateCategoryRequest,
};
use crate::AppState;

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

// ============================================================================
// Category endpoints
// ============================================================================

pub async fn create_category(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCategoryRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let cat = CategoryRepo::create(&state.pool, &req).await.map_err(map_error)?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(cat).unwrap())))
}

pub async fn update_category(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCategoryRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let cat = CategoryRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(cat).unwrap()))
}

pub async fn deactivate_category(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let cat = CategoryRepo::deactivate(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(cat).unwrap()))
}

pub async fn get_category(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let cat = CategoryRepo::find_by_id(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(AssetError::NotFound))?;
    Ok(Json(serde_json::to_value(cat).unwrap()))
}

pub async fn list_categories(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let cats = CategoryRepo::list(&state.pool, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(cats).unwrap()))
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
    Json(req): Json<CreateAssetRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let asset = AssetRepo::create(&state.pool, &req).await.map_err(map_error)?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(asset).unwrap())))
}

pub async fn update_asset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAssetRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let asset = AssetRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(asset).unwrap()))
}

pub async fn deactivate_asset(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let asset = AssetRepo::deactivate(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(asset).unwrap()))
}

pub async fn get_asset(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let asset = AssetRepo::find_by_id(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(AssetError::NotFound))?;
    Ok(Json(serde_json::to_value(asset).unwrap()))
}

pub async fn list_assets(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    Query(query): Query<ListAssetsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let assets = AssetRepo::list(&state.pool, &tenant_id, query.status.as_deref())
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(assets).unwrap()))
}

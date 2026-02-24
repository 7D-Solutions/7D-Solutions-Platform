//! Maintainable asset HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/maintenance/assets           — create asset
//!   GET   /api/maintenance/assets           — list assets (filterable)
//!   GET   /api/maintenance/assets/:id       — get asset detail
//!   PATCH /api/maintenance/assets/:id       — update asset

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::assets::{
    AssetError, AssetRepo, CreateAssetRequest, ListAssetsQuery, UpdateAssetRequest,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct TenantQuery {
    pub tenant_id: String,
}

fn asset_error_response(err: AssetError) -> impl IntoResponse {
    match err {
        AssetError::DuplicateTag(tag, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_asset_tag",
                "message": format!("Asset tag '{}' already exists for tenant '{}'", tag, tenant)
            })),
        ),
        AssetError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Asset not found" })),
        ),
        AssetError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        AssetError::Database(e) => {
            tracing::error!(error = %e, "asset database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/maintenance/assets
pub async fn create_asset(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAssetRequest>,
) -> impl IntoResponse {
    match AssetRepo::create(&state.pool, &req).await {
        Ok(asset) => (StatusCode::CREATED, Json(json!(asset))).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets
pub async fn list_assets(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListAssetsQuery>,
) -> impl IntoResponse {
    match AssetRepo::list(&state.pool, &q).await {
        Ok(resp) => (StatusCode::OK, Json(json!(resp))).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets/:id
pub async fn get_asset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match AssetRepo::find_by_id(&state.pool, id, &q.tenant_id).await {
        Ok(Some(asset)) => (StatusCode::OK, Json(json!(asset))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Asset not found" })),
        )
            .into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

/// PATCH /api/maintenance/assets/:id
pub async fn update_asset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
    Json(req): Json<UpdateAssetRequest>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match AssetRepo::update(&state.pool, id, &q.tenant_id, &req).await {
        Ok(asset) => (StatusCode::OK, Json(json!(asset))).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

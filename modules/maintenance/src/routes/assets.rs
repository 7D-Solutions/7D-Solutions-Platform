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
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::assets::{
    AssetError, AssetRepo, CreateAssetRequest, ListAssetsQuery, UpdateAssetRequest,
};
use crate::AppState;
use super::ErrorBody;

#[derive(Debug, Deserialize)]
pub struct ListAssetsParams {
    pub asset_type: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn asset_error_response(err: AssetError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        AssetError::DuplicateTag(tag, tenant) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_asset_tag",
                &format!("Asset tag '{}' already exists for tenant '{}'", tag, tenant),
            )),
        ),
        AssetError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Asset not found")),
        ),
        AssetError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        AssetError::Database(e) => {
            tracing::error!(error = %e, "asset database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/assets
pub async fn create_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateAssetRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match AssetRepo::create(&state.pool, &req).await {
        Ok(asset) => (StatusCode::CREATED, Json(json!(asset))).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets
pub async fn list_assets(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListAssetsParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListAssetsQuery {
        tenant_id,
        asset_type: params.asset_type,
        status: params.status,
        limit: params.limit,
        offset: params.offset,
    };
    match AssetRepo::list(&state.pool, &q).await {
        Ok(resp) => (StatusCode::OK, Json(json!(resp))).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets/:id
pub async fn get_asset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match AssetRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(asset)) => (StatusCode::OK, Json(json!(asset))).into_response(),
        Ok(None) => asset_error_response(AssetError::NotFound).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

/// PATCH /api/maintenance/assets/:id
pub async fn update_asset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<UpdateAssetRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match AssetRepo::update(&state.pool, id, &tenant_id, &req).await {
        Ok(asset) => (StatusCode::OK, Json(json!(asset))).into_response(),
        Err(e) => asset_error_response(e).into_response(),
    }
}

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new("unauthorized", "Missing or invalid authentication")),
        )),
    }
}

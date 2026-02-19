//! HTTP handlers for asset disposals and impairments.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::disposals::{DisposeAssetRequest, DisposalError, DisposalService};
use crate::AppState;

fn map_error(e: DisposalError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, msg) = match &e {
        DisposalError::AssetNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
        DisposalError::CategoryNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
        DisposalError::InvalidState(_) => (StatusCode::CONFLICT, e.to_string()),
        DisposalError::Validation(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        DisposalError::Database(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error".to_string(),
        ),
    };
    (status, Json(serde_json::json!({ "error": msg })))
}

/// POST /api/fixed-assets/disposals — Dispose or impair an asset. Idempotent.
pub async fn dispose_asset(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DisposeAssetRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let disposal = DisposalService::dispose(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(disposal).unwrap()),
    ))
}

/// GET /api/fixed-assets/disposals/:tenant_id — List all disposals.
pub async fn list_disposals(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let disposals = DisposalService::list(&state.pool, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(disposals).unwrap()))
}

/// GET /api/fixed-assets/disposals/:tenant_id/:id — Fetch a single disposal.
pub async fn get_disposal(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let disposal = DisposalService::get(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(DisposalError::AssetNotFound(id)))?;
    Ok(Json(serde_json::to_value(disposal).unwrap()))
}

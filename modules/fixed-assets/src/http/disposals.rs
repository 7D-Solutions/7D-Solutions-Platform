//! HTTP handlers for asset disposals and impairments.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::disposals::{DisposalError, DisposalService, DisposeAssetRequest};
use crate::AppState;

use super::helpers::tenant::extract_tenant;

fn map_error(e: DisposalError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, code, msg) = match &e {
        DisposalError::AssetNotFound(_) => (StatusCode::NOT_FOUND, "not_found", e.to_string()),
        DisposalError::CategoryNotFound(_) => (StatusCode::NOT_FOUND, "not_found", e.to_string()),
        DisposalError::InvalidState(_) => (StatusCode::CONFLICT, "invalid_state", e.to_string()),
        DisposalError::Validation(_) => {
            (StatusCode::BAD_REQUEST, "validation_error", e.to_string())
        }
        DisposalError::Database(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal error".to_string(),
        ),
    };
    (
        status,
        Json(serde_json::json!({ "error": code, "message": msg })),
    )
}

fn map_internal_error<E: std::fmt::Display>(e: E) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!(error = %e, "Internal error during serialization");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal_error", "message": "Internal error" })),
    )
}

/// POST /api/fixed-assets/disposals — Dispose or impair an asset. Idempotent.
pub async fn dispose_asset(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<DisposeAssetRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let disposal = DisposalService::dispose(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(disposal).map_err(map_internal_error)?),
    ))
}

/// GET /api/fixed-assets/disposals — List all disposals.
pub async fn list_disposals(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let disposals = DisposalService::list(&state.pool, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(
        serde_json::to_value(disposals).map_err(map_internal_error)?,
    ))
}

/// GET /api/fixed-assets/disposals/:id — Fetch a single disposal.
pub async fn get_disposal(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let disposal = DisposalService::get(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(DisposalError::AssetNotFound(id)))?;
    Ok(Json(
        serde_json::to_value(disposal).map_err(map_internal_error)?,
    ))
}

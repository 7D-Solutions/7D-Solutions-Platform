//! HTTP handlers for depreciation schedule generation and run execution.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::depreciation::{
    CreateRunRequest, DepreciationError, DepreciationService, GenerateScheduleRequest,
};
use crate::AppState;

use super::helpers::tenant::extract_tenant;

// ============================================================================
// Error mapping
// ============================================================================

fn map_error(e: DepreciationError) -> (StatusCode, Json<serde_json::Value>) {
    let (status, msg) = match &e {
        DepreciationError::AssetNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
        DepreciationError::AssetNotInService(_) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        DepreciationError::UnsupportedMethod(_) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        DepreciationError::Validation(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        DepreciationError::Database(_) => (
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
// Schedule endpoints
// ============================================================================

/// POST /api/fixed-assets/depreciation/schedule
///
/// Generate (or refresh) the straight-line depreciation schedule for a single asset.
/// Idempotent — safe to call multiple times.
pub async fn generate_schedule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<GenerateScheduleRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let schedules = DepreciationService::generate_schedule(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(schedules).map_err(map_internal_error)?),
    ))
}

// ============================================================================
// Run endpoints
// ============================================================================

/// POST /api/fixed-assets/depreciation/runs
///
/// Execute a depreciation run: posts all unposted periods up to as_of_date.
/// Idempotent — re-running for the same period skips already-posted entries.
pub async fn create_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    let run = DepreciationService::run(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(run).map_err(map_internal_error)?),
    ))
}

/// GET /api/fixed-assets/depreciation/runs
///
/// List all depreciation runs for a tenant, newest first.
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let runs = DepreciationService::list_runs(&state.pool, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(runs).map_err(map_internal_error)?))
}

/// GET /api/fixed-assets/depreciation/runs/:id
///
/// Fetch a single depreciation run.
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let tenant_id = extract_tenant(&claims)?;

    let run = DepreciationService::get_run(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(DepreciationError::AssetNotFound(id)))?;
    Ok(Json(serde_json::to_value(run).map_err(map_internal_error)?))
}

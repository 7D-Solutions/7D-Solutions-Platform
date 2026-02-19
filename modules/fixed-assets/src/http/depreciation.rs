//! HTTP handlers for depreciation schedule generation and run execution.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::depreciation::{
    CreateRunRequest, DepreciationError, DepreciationService, GenerateScheduleRequest,
};
use crate::AppState;

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

// ============================================================================
// Schedule endpoints
// ============================================================================

/// POST /api/fixed-assets/depreciation/schedule
///
/// Generate (or refresh) the straight-line depreciation schedule for a single asset.
/// Idempotent — safe to call multiple times.
pub async fn generate_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GenerateScheduleRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let schedules = DepreciationService::generate_schedule(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(schedules).unwrap()),
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
    Json(req): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let run = DepreciationService::run(&state.pool, &req)
        .await
        .map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(run).unwrap()),
    ))
}

/// GET /api/fixed-assets/depreciation/runs/:tenant_id
///
/// List all depreciation runs for a tenant, newest first.
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let runs = DepreciationService::list_runs(&state.pool, &tenant_id)
        .await
        .map_err(map_error)?;
    Ok(Json(serde_json::to_value(runs).unwrap()))
}

/// GET /api/fixed-assets/depreciation/runs/:tenant_id/:id
///
/// Fetch a single depreciation run.
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    Path((tenant_id, id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let run = DepreciationService::get_run(&state.pool, id, &tenant_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| map_error(DepreciationError::AssetNotFound(id)))?;
    Ok(Json(serde_json::to_value(run).unwrap()))
}

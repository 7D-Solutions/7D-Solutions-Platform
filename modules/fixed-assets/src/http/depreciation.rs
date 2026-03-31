//! HTTP handlers for depreciation schedule generation and run execution.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::depreciation::{
    CreateRunRequest, DepreciationService, GenerateScheduleRequest,
};
use crate::AppState;

use super::helpers::tenant::{extract_tenant, with_request_id};

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
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<GenerateScheduleRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match DepreciationService::generate_schedule(&state.pool, &req).await {
        Ok(schedules) => (StatusCode::CREATED, Json(schedules)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
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
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateRunRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match DepreciationService::run(&state.pool, &req).await {
        Ok(run) => (StatusCode::CREATED, Json(run)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/fixed-assets/depreciation/runs
///
/// List all depreciation runs for a tenant, newest first.
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match DepreciationService::list_runs(&state.pool, &tenant_id).await {
        Ok(runs) => {
            let total = runs.len() as i64;
            let resp = PaginatedResponse::new(runs, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/fixed-assets/depreciation/runs/:id
///
/// Fetch a single depreciation run.
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match DepreciationService::get_run(&state.pool, id, &tenant_id).await {
        Ok(Some(run)) => Json(run).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Depreciation run {} not found", id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

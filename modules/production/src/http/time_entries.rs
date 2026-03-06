use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::time_entries::{ManualEntryRequest, StartTimerRequest, StopTimerRequest, TimeEntryError, TimeEntryRepo},
    AppState,
};

fn time_entry_error_response(err: TimeEntryError) -> impl IntoResponse {
    match err {
        TimeEntryError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Time entry not found" })),
        )
            .into_response(),
        TimeEntryError::WorkOrderNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        )
            .into_response(),
        TimeEntryError::OperationNotFound => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "operation_not_found",
                "message": "Operation not found or does not belong to work order"
            })),
        )
            .into_response(),
        TimeEntryError::AlreadyStopped => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_stopped",
                "message": "Timer has already been stopped"
            })),
        )
            .into_response(),
        TimeEntryError::InvalidTimeRange => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "invalid_time_range",
                "message": "End time must be after start time"
            })),
        )
            .into_response(),
        TimeEntryError::Database(e) => {
            tracing::error!(error = %e, "time entry database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// POST /api/production/time-entries/start
pub async fn start_timer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<StartTimerRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match TimeEntryRepo::start_timer(&state.pool, &req, &tenant_id, &corr, None).await {
        Ok(entry) => (StatusCode::CREATED, Json(json!(entry))).into_response(),
        Err(e) => time_entry_error_response(e).into_response(),
    }
}

/// POST /api/production/time-entries/:id/stop
pub async fn stop_timer(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(req): Json<StopTimerRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match TimeEntryRepo::stop_timer(&state.pool, id, &req, &tenant_id, &corr, None).await {
        Ok(entry) => (StatusCode::OK, Json(json!(entry))).into_response(),
        Err(e) => time_entry_error_response(e).into_response(),
    }
}

/// POST /api/production/time-entries/manual
pub async fn manual_entry(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<ManualEntryRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match TimeEntryRepo::manual_entry(&state.pool, &req, &tenant_id, &corr, None).await {
        Ok(entry) => (StatusCode::CREATED, Json(json!(entry))).into_response(),
        Err(e) => time_entry_error_response(e).into_response(),
    }
}

/// GET /api/production/work-orders/:id/time-entries
pub async fn list_time_entries(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(wo_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match TimeEntryRepo::list_by_work_order(&state.pool, wo_id, &tenant_id).await {
        Ok(entries) => (StatusCode::OK, Json(json!(entries))).into_response(),
        Err(e) => time_entry_error_response(e).into_response(),
    }
}

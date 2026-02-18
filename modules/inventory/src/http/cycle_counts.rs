//! Cycle count task HTTP handler.
//!
//! Endpoint:
//!   POST /api/inventory/cycle-count-tasks — create a task + snapshot lines
//!
//! Full scope:   lines auto-populated from on-hand projection at the location.
//! Partial scope: lines built from caller-specified item_ids.
//!
//! Stock changes are NOT applied here. The submit endpoint (bd-1q0j) applies
//! adjustments after the counted_qty is filled in.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use std::sync::Arc;

use crate::{
    domain::cycle_count::task_service::{
        create_cycle_count_task, CreateTaskRequest, TaskError,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn task_error_response(err: TaskError) -> impl IntoResponse {
    match err {
        TaskError::MissingTenant => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "validation_error",
                "message": "tenant_id is required"
            })),
        )
            .into_response(),

        TaskError::EmptyPartialItemList => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "validation_error",
                "message": "partial scope requires at least one item_id"
            })),
        )
            .into_response(),

        TaskError::LocationNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "location_not_found",
                "message": "Location not found, inactive, or does not belong to this tenant/warehouse"
            })),
        )
            .into_response(),

        TaskError::Database(e) => {
            tracing::error!(error = %e, "database error creating cycle count task");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/cycle-count-tasks
///
/// Creates a cycle count task with snapshotted lines.
/// Returns 201 Created with the full task including all lines.
pub async fn post_cycle_count_task(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    match create_cycle_count_task(&state.pool, &req).await {
        Ok(result) => (StatusCode::CREATED, Json(result)).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}

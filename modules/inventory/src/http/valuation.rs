//! Valuation snapshot HTTP handlers.
//!
//! Endpoint:
//!   POST /api/inventory/valuation-snapshots
//!     — build a point-in-time valuation snapshot from FIFO layers
//!
//! Returns 201 Created on first call; 200 OK on idempotent replay.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use std::sync::Arc;

use crate::{
    domain::valuation::snapshot_service::{
        create_valuation_snapshot, CreateSnapshotRequest, SnapshotError,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn snapshot_error_response(err: SnapshotError) -> impl IntoResponse {
    match err {
        SnapshotError::MissingTenant | SnapshotError::MissingIdempotencyKey => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": err.to_string() })),
        )
            .into_response(),

        SnapshotError::ConcurrentSnapshot => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "concurrent_snapshot",
                "message": err.to_string()
            })),
        )
            .into_response(),

        SnapshotError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": err.to_string()
            })),
        )
            .into_response(),

        SnapshotError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in valuation snapshot");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
                .into_response()
        }

        SnapshotError::Database(e) => {
            tracing::error!(error = %e, "database error creating valuation snapshot");
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

/// POST /api/inventory/valuation-snapshots
///
/// Creates a valuation snapshot from remaining FIFO layers as-of `req.as_of`.
/// Returns 201 on creation; 200 on idempotent replay.
pub async fn post_valuation_snapshot(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSnapshotRequest>,
) -> impl IntoResponse {
    match create_valuation_snapshot(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => snapshot_error_response(err).into_response(),
    }
}

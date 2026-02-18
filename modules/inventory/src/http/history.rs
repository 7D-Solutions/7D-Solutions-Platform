//! Movement history HTTP handler.
//!
//! GET /api/inventory/items/{item_id}/history?tenant_id=...
//! GET /api/inventory/items/{item_id}/history?tenant_id=...&location_id=<uuid>
//!
//! Returns an ordered list of ledger movements for the item, tenant-scoped
//! and optionally filtered to a specific location.  Ordering is deterministic:
//! posted_at ASC, ledger id ASC.
//!
//! This is a read-only, index-backed endpoint.  It never modifies state.

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

use crate::{domain::history::query::query_movement_history, AppState};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub tenant_id: String,
    pub location_id: Option<Uuid>,
}

// ============================================================================
// Handler
// ============================================================================

/// GET /api/inventory/items/{item_id}/history?tenant_id=...&location_id=...
///
/// Returns all ledger movements for the item scoped to the tenant.
/// When `location_id` is supplied, only movements that touched that location
/// are returned.  When omitted, all movements (across all locations) are
/// returned in chronological order.
///
/// Response: 200 `{ "item_id", "tenant_id", "location_id", "movements": [...] }`
pub async fn get_movement_history(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> impl IntoResponse {
    match query_movement_history(&state.pool, &q.tenant_id, item_id, q.location_id).await {
        Ok(movements) => (
            StatusCode::OK,
            Json(json!({
                "item_id": item_id,
                "tenant_id": q.tenant_id,
                "location_id": q.location_id,
                "movements": movements
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(
                error = %e,
                item_id = %item_id,
                "database error fetching movement history"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

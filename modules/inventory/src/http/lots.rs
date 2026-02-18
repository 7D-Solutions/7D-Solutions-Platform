//! Lot listing HTTP handler.
//!
//! Endpoint:
//!   GET /api/inventory/items/{item_id}/lots?tenant_id=... — list lots for item

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

use crate::{domain::lots_serials::queries::list_lots_for_item, AppState};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct TenantQuery {
    pub tenant_id: String,
}

// ============================================================================
// Handler
// ============================================================================

/// GET /api/inventory/items/{item_id}/lots?tenant_id=...
///
/// Returns all lots for the item scoped to the tenant.
/// Responds with 200 + `{ "lots": [...] }`. Empty array when no lots exist.
pub async fn get_lots_for_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    match list_lots_for_item(&state.pool, &q.tenant_id, item_id).await {
        Ok(lots) => (StatusCode::OK, Json(json!({ "lots": lots }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, item_id = %item_id, "database error listing lots");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

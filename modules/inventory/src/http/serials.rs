//! Serial instance listing HTTP handler.
//!
//! Endpoint:
//!   GET /api/inventory/items/{item_id}/serials?tenant_id=... — list serials for item

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

use crate::{domain::lots_serials::queries::list_serials_for_item, AppState};

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

/// GET /api/inventory/items/{item_id}/serials?tenant_id=...
///
/// Returns all serial instances for the item scoped to the tenant.
/// Responds with 200 + `{ "serials": [...] }`. Empty array when none exist.
pub async fn get_serials_for_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    match list_serials_for_item(&state.pool, &q.tenant_id, item_id).await {
        Ok(serials) => (StatusCode::OK, Json(json!({ "serials": serials }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, item_id = %item_id, "database error listing serials");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

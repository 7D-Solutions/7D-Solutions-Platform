//! Lot listing HTTP handler.
//!
//! Endpoint:
//!   GET /api/inventory/items/{item_id}/lots — list lots for item (tenant from JWT)

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

use crate::{domain::lots_serials::queries::list_lots_for_item, AppState};

// ============================================================================
// Handler
// ============================================================================

/// GET /api/inventory/items/{item_id}/lots
///
/// Returns all lots for the item scoped to the tenant (from JWT).
/// Responds with 200 + `{ "lots": [...] }`. Empty array when no lots exist.
pub async fn get_lots_for_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    match list_lots_for_item(&state.pool, &tenant_id, item_id).await {
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

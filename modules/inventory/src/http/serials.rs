//! Serial instance listing HTTP handler.
//!
//! Endpoint:
//!   GET /api/inventory/items/{item_id}/serials — list serials for item (tenant from JWT)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::{domain::lots_serials::queries::list_serials_for_item, AppState};

// ============================================================================
// Handler
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/serials",
    tag = "Lots & Serials",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Serial instances for item", body = serde_json::Value),
    ),
    security(("bearer" = [])),
)]
pub async fn get_serials_for_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match list_serials_for_item(&state.pool, &tenant_id, item_id).await {
        Ok(serials) => (StatusCode::OK, Json(json!({ "serials": serials }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, item_id = %item_id, "database error listing serials");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

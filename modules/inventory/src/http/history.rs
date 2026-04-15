//! Movement history HTTP handler.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{domain::history::query::query_movement_history, AppState};
use platform_sdk::extract_tenant;

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub location_id: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/history",
    tag = "History",
    params(
        ("item_id" = Uuid, Path, description = "Item ID"),
        ("location_id" = Option<Uuid>, Query, description = "Filter by location"),
    ),
    responses(
        (status = 200, description = "Movement history", body = serde_json::Value),
    ),
    security(("bearer" = [])),
)]
pub async fn get_movement_history(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<HistoryQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match query_movement_history(&state.pool, &tenant_id, item_id, q.location_id).await {
        Ok(movements) => (
            StatusCode::OK,
            Json(json!({
                "item_id": item_id,
                "tenant_id": tenant_id,
                "location_id": q.location_id,
                "movements": movements
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, item_id = %item_id, "database error fetching movement history");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

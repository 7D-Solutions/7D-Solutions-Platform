//! Lot listing HTTP handler.
//!
//! Endpoint:
//!   GET /api/inventory/items/{item_id}/lots — list lots for item (tenant from JWT)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::{domain::lots_serials::queries::list_lots_for_item, AppState};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListLotsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

// ============================================================================
// Handler
// ============================================================================

/// GET /api/inventory/items/{item_id}/lots
///
/// Returns lots for the item scoped to the tenant (from JWT).
/// Returns `PaginatedResponse` envelope.
pub async fn get_lots_for_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListLotsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match list_lots_for_item(&state.pool, &tenant_id, item_id).await {
        Ok(all_lots) => {
            let total = all_lots.len() as i64;
            let page_size = q.limit.clamp(1, 200);
            let offset = q.offset.max(0);
            let page = (offset / page_size) + 1;
            let lots: Vec<_> = all_lots
                .into_iter()
                .skip(offset as usize)
                .take(page_size as usize)
                .collect();
            let resp = PaginatedResponse::new(lots, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, item_id = %item_id, "database error listing lots");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

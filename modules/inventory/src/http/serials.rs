//! Serial instance listing HTTP handler.
//!
//! Endpoint:
//!   GET /api/inventory/items/{item_id}/serials — list serials for item (tenant from JWT)

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

use super::tenant::with_request_id;
use crate::{domain::lots_serials::queries::list_serials_for_item, AppState};
use platform_sdk::extract_tenant;

#[derive(Debug, Deserialize)]
pub struct ListSerialsQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

// ============================================================================
// Handler
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/serials",
    tag = "Lots & Serials",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Paginated serial list", body = PaginatedResponse<crate::domain::lots_serials::models::InventorySerialInstance>),
    ),
    security(("bearer" = [])),
)]
pub async fn get_serials_for_item(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListSerialsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match list_serials_for_item(&state.pool, &tenant_id, item_id).await {
        Ok(all_serials) => {
            let total = all_serials.len() as i64;
            let page_size = q.page_size.clamp(1, 200);
            let page = q.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let serials: Vec<_> = all_serials
                .into_iter()
                .skip(offset)
                .take(page_size as usize)
                .collect();
            let resp = PaginatedResponse::new(serials, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, item_id = %item_id, "database error listing serials");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

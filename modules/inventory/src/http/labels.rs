//! Label generation HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items/:item_id/labels           — generate a label
//!   GET   /api/inventory/items/:item_id/labels           — list labels for item
//!   GET   /api/inventory/labels/:label_id                — get single label

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

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::labels::{generate_label, get_label, list_labels, GenerateLabelRequest, Label},
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListLabelsQuery {
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
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/items/{item_id}/labels",
    tag = "Labels",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    request_body = GenerateLabelRequest,
    responses(
        (status = 201, description = "Label generated", body = Label),
        (status = 200, description = "Idempotency replay", body = Label),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_generate_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<GenerateLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    req.item_id = item_id;

    if req.actor_id.is_none() {
        if let Some(Extension(c)) = &claims {
            req.actor_id = Some(c.user_id);
        }
    }

    match generate_label(&state.pool, &req).await {
        Ok((label, false)) => (StatusCode::CREATED, Json(label)).into_response(),
        Ok((label, true)) => (StatusCode::OK, Json(label)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/labels",
    tag = "Labels",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Paginated label list", body = PaginatedResponse<Label>),
    ),
    security(("bearer" = [])),
)]
pub async fn get_list_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Query(q): Query<ListLabelsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match list_labels(&state.pool, &tenant_id, item_id).await {
        Ok(all_labels) => {
            let total = all_labels.len() as i64;
            let page_size = q.page_size.clamp(1, 200);
            let page = q.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let labels: Vec<_> = all_labels
                .into_iter()
                .skip(offset)
                .take(page_size as usize)
                .collect();
            let resp = PaginatedResponse::new(labels, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/labels/{label_id}",
    tag = "Labels",
    params(("label_id" = Uuid, Path, description = "Label ID")),
    responses(
        (status = 200, description = "Label details", body = Label),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_label_by_id(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(label_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match get_label(&state.pool, &tenant_id, label_id).await {
        Ok(Some(label)) => (StatusCode::OK, Json(label)).into_response(),
        Ok(None) => {
            with_request_id(ApiError::not_found("Label not found"), &tracing_ctx).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

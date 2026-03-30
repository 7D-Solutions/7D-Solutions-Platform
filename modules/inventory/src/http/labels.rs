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

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::labels::{generate_label, get_label, list_labels, GenerateLabelRequest},
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListLabelsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/items/:item_id/labels
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

/// GET /api/inventory/items/:item_id/labels
///
/// Lists labels for an item. Returns `PaginatedResponse` envelope.
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
            let page_size = q.limit.clamp(1, 200);
            let offset = q.offset.max(0);
            let page = (offset / page_size) + 1;
            let labels: Vec<_> = all_labels
                .into_iter()
                .skip(offset as usize)
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

/// GET /api/inventory/labels/:label_id
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

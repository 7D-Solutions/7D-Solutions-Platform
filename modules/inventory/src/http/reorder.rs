//! Reorder policy HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/reorder-policies              — create policy
//!   GET  /api/inventory/reorder-policies/:id          — get policy
//!   PUT  /api/inventory/reorder-policies/:id          — update policy
//!   GET  /api/inventory/items/:item_id/reorder-policies — list for item

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
    domain::reorder::models::{
        CreateReorderPolicyRequest, ReorderPolicyRepo, UpdateReorderPolicyRequest,
    },
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListPoliciesQuery {
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

/// POST /api/inventory/reorder-policies
pub async fn post_reorder_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateReorderPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match ReorderPolicyRepo::create(&state.pool, &req).await {
        Ok(policy) => (StatusCode::CREATED, Json(policy)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/inventory/reorder-policies/:id
pub async fn get_reorder_policy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match ReorderPolicyRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(policy)) => (StatusCode::OK, Json(policy)).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found("Reorder policy not found"),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// PUT /api/inventory/reorder-policies/:id
pub async fn put_reorder_policy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateReorderPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match ReorderPolicyRepo::update(&state.pool, id, &req).await {
        Ok(policy) => (StatusCode::OK, Json(policy)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/inventory/items/:item_id/reorder-policies
///
/// Lists all reorder policies for an item. Returns `PaginatedResponse` envelope.
pub async fn list_reorder_policies(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListPoliciesQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match ReorderPolicyRepo::list_for_item(&state.pool, &tenant_id, item_id).await {
        Ok(all_policies) => {
            let total = all_policies.len() as i64;
            let page_size = q.limit.clamp(1, 200);
            let offset = q.offset.max(0);
            let page = (offset / page_size) + 1;
            let policies: Vec<_> = all_policies
                .into_iter()
                .skip(offset as usize)
                .take(page_size as usize)
                .collect();
            let resp = PaginatedResponse::new(policies, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_reorder_routes_compile() {}
}

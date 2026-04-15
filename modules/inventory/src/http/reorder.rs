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

use super::tenant::with_request_id;
use crate::{
    domain::reorder::models::{
        CreateReorderPolicyRequest, ReorderPolicy, ReorderPolicyRepo, UpdateReorderPolicyRequest,
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListPoliciesQuery {
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
    path = "/api/inventory/reorder-policies",
    tag = "Reorder Policies",
    request_body = CreateReorderPolicyRequest,
    responses(
        (status = 201, description = "Reorder policy created", body = ReorderPolicy),
        (status = 409, description = "Duplicate policy", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get,
    path = "/api/inventory/reorder-policies/{id}",
    tag = "Reorder Policies",
    params(("id" = Uuid, Path, description = "Reorder policy ID")),
    responses(
        (status = 200, description = "Reorder policy details", body = ReorderPolicy),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    put,
    path = "/api/inventory/reorder-policies/{id}",
    tag = "Reorder Policies",
    params(("id" = Uuid, Path, description = "Reorder policy ID")),
    request_body = UpdateReorderPolicyRequest,
    responses(
        (status = 200, description = "Reorder policy updated", body = ReorderPolicy),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get,
    path = "/api/inventory/items/{item_id}/reorder-policies",
    tag = "Reorder Policies",
    params(("item_id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Paginated reorder policy list", body = PaginatedResponse<ReorderPolicy>),
    ),
    security(("bearer" = [])),
)]
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
            let page_size = q.page_size.clamp(1, 200);
            let page = q.page.max(1);
            let offset = ((page - 1) * page_size) as usize;
            let policies: Vec<_> = all_policies
                .into_iter()
                .skip(offset)
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

//! Item master HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items            — create item
//!   GET   /api/inventory/items/:id        — get item
//!   PUT   /api/inventory/items/:id        — update item
//!   GET   /api/inventory/items            — list items (paginated)
//!   POST  /api/inventory/items/:id/deactivate — soft-delete item
//!
//! Tenant identity derived from JWT `VerifiedClaims`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::items::{CreateItemRequest, Item, ItemRepo, ListItemsQuery, UpdateItemRequest},
    AppState,
};

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/items",
    tag = "Items",
    request_body = CreateItemRequest,
    responses(
        (status = 201, description = "Item created", body = Item),
        (status = 409, description = "SKU already exists for tenant", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateItemRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match ItemRepo::create(&state.pool, &req).await {
        Ok(item) => (StatusCode::CREATED, Json(item)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items/{id}",
    tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Item details", body = Item),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ItemRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(item)) => (StatusCode::OK, Json(item)).into_response(),
        Ok(None) => {
            with_request_id(ApiError::not_found("Item not found"), &tracing_ctx).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    put,
    path = "/api/inventory/items/{id}",
    tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    request_body = UpdateItemRequest,
    responses(
        (status = 200, description = "Item updated", body = Item),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation failure", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn update_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateItemRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match ItemRepo::update(&state.pool, id, &req).await {
        Ok(item) => (StatusCode::OK, Json(item)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/items",
    tag = "Items",
    params(ListItemsQuery),
    responses(
        (status = 200, description = "Paginated item list", body = PaginatedResponse<Item>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_items(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListItemsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ItemRepo::list(&state.pool, &tenant_id, &query).await {
        Ok((items, total)) => {
            let page_size = query.page_size.clamp(1, 200) as i64;
            let page = query.page.max(1) as i64;
            let resp = PaginatedResponse::new(items, page, page_size, total);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/items/{id}/deactivate",
    tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Item deactivated (idempotent)", body = Item),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn deactivate_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ItemRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(item) => (StatusCode::OK, Json(item)).into_response(),
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
    /// Route handler tests that require a real DB live in the integration test suite
    /// (cargo test -p inventory). Pure unit tests for request parsing are kept here.

    #[test]
    fn placeholder_route_module_compiles() {
        // Ensures the module compiles cleanly as a unit test target.
    }
}

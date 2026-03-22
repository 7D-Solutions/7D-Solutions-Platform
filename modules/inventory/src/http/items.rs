//! Item master HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items            — create item
//!   GET   /api/inventory/items/:id        — get item
//!   PUT   /api/inventory/items/:id        — update item
//!   POST  /api/inventory/items/:id/deactivate — soft-delete item
//!
//! Tenant identity derived from JWT `VerifiedClaims`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::items::{CreateItemRequest, ItemError, ItemRepo, ListItemsQuery, UpdateItemRequest},
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn item_error_response(err: ItemError) -> impl IntoResponse {
    match err {
        ItemError::DuplicateSku(sku, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_sku",
                "message": format!("SKU '{}' already exists for tenant '{}'", sku, tenant)
            })),
        ),
        ItemError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Item not found" })),
        ),
        ItemError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ItemError::Database(e) => {
            tracing::error!(error = %e, "item database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/items
///
/// Create a new item. Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
/// Returns 201 Created with the item on success,
/// 409 Conflict if the SKU already exists for the tenant,
/// 422 Unprocessable Entity on validation failure.
pub async fn create_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateItemRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match ItemRepo::create(&state.pool, &req).await {
        Ok(item) => (StatusCode::CREATED, Json(json!(item))).into_response(),
        Err(e) => item_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items/:id
///
/// Fetch an item by id scoped to tenant (from JWT).
/// Returns 200 OK with full item (including tracking_mode) on success,
/// 404 Not Found if not found.
pub async fn get_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ItemRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(item)) => (StatusCode::OK, Json(json!(item))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Item not found" })),
        )
            .into_response(),
        Err(e) => item_error_response(e).into_response(),
    }
}

/// PUT /api/inventory/items/:id
///
/// Update mutable fields. Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
/// Returns 200 OK with updated item on success,
/// 404 Not Found if item doesn't exist for tenant,
/// 422 on validation failure.
pub async fn update_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateItemRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match ItemRepo::update(&state.pool, id, &req).await {
        Ok(item) => (StatusCode::OK, Json(json!(item))).into_response(),
        Err(e) => item_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items
///
/// List items with optional search, filtering, and pagination.
/// Tenant derived from JWT VerifiedClaims.
pub async fn list_items(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListItemsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ItemRepo::list(&state.pool, &tenant_id, &query).await {
        Ok((items, total)) => (
            StatusCode::OK,
            Json(json!({
                "items": items,
                "total": total,
                "limit": query.limit.clamp(1, 200),
                "offset": query.offset.max(0),
            })),
        )
            .into_response(),
        Err(e) => item_error_response(e).into_response(),
    }
}

/// POST /api/inventory/items/:id/deactivate
///
/// Soft-delete an item. Tenant derived from JWT VerifiedClaims.
/// Idempotent — already-inactive items return 200.
pub async fn deactivate_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match ItemRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(item) => (StatusCode::OK, Json(json!(item))).into_response(),
        Err(e) => item_error_response(e).into_response(),
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

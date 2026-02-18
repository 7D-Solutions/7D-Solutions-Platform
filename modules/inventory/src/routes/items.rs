//! Item master HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items            — create item
//!   PUT   /api/inventory/items/:id        — update item
//!   POST  /api/inventory/items/:id/deactivate — soft-delete item

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::items::{CreateItemRequest, ItemError, ItemRepo, UpdateItemRequest},
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
/// Create a new item. Returns 201 Created with the item on success,
/// 409 Conflict if the SKU already exists for the tenant,
/// 422 Unprocessable Entity on validation failure.
pub async fn create_item(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateItemRequest>,
) -> impl IntoResponse {
    match ItemRepo::create(&state.pool, &req).await {
        Ok(item) => (StatusCode::CREATED, Json(json!(item))).into_response(),
        Err(e) => item_error_response(e).into_response(),
    }
}

/// PUT /api/inventory/items/:id
///
/// Update mutable fields. Returns 200 OK with updated item on success,
/// 404 Not Found if item doesn't exist for tenant,
/// 422 on validation failure.
pub async fn update_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateItemRequest>,
) -> impl IntoResponse {
    match ItemRepo::update(&state.pool, id, &req).await {
        Ok(item) => (StatusCode::OK, Json(json!(item))).into_response(),
        Err(e) => item_error_response(e).into_response(),
    }
}

/// POST /api/inventory/items/:id/deactivate
///
/// Soft-delete an item. Idempotent — already-inactive items return 200.
/// Body must contain `tenant_id` for cross-tenant safety.
pub async fn deactivate_item(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let tenant_id = match body.get("tenant_id").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
            )
                .into_response();
        }
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

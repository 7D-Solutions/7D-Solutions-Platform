//! Location HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/locations                  — create location
//!   GET   /api/inventory/locations/:id              — get location
//!   PUT   /api/inventory/locations/:id              — update location
//!   POST  /api/inventory/locations/:id/deactivate   — soft-delete location
//!   GET   /api/inventory/warehouses/:wid/locations  — list by warehouse

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::locations::{
        CreateLocationRequest, LocationError, LocationRepo, UpdateLocationRequest,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn location_error_response(err: LocationError) -> impl IntoResponse {
    match err {
        LocationError::DuplicateCode(code, wid, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_code",
                "message": format!(
                    "Location code '{}' already exists for warehouse '{}' in tenant '{}'",
                    code, wid, tenant
                )
            })),
        )
            .into_response(),
        LocationError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Location not found" })),
        )
            .into_response(),
        LocationError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        LocationError::Database(e) => {
            tracing::error!(error = %e, "location database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Deserialize)]
pub struct TenantQuery {
    pub tenant_id: String,
}

#[derive(Deserialize)]
pub struct WarehouseQuery {
    pub tenant_id: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/locations
pub async fn create_location(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateLocationRequest>,
) -> impl IntoResponse {
    match LocationRepo::create(&state.pool, &req).await {
        Ok(loc) => (StatusCode::CREATED, Json(json!(loc))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// GET /api/inventory/locations/:id?tenant_id=...
pub async fn get_location(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }
    match LocationRepo::find_by_id(&state.pool, id, &q.tenant_id).await {
        Ok(Some(loc)) => (StatusCode::OK, Json(json!(loc))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Location not found" })),
        )
            .into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// PUT /api/inventory/locations/:id
pub async fn update_location(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLocationRequest>,
) -> impl IntoResponse {
    match LocationRepo::update(&state.pool, id, &req).await {
        Ok(loc) => (StatusCode::OK, Json(json!(loc))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// POST /api/inventory/locations/:id/deactivate
pub async fn deactivate_location(
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
    match LocationRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(loc) => (StatusCode::OK, Json(json!(loc))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// GET /api/inventory/warehouses/:warehouse_id/locations?tenant_id=...
pub async fn list_locations(
    State(state): State<Arc<AppState>>,
    Path(warehouse_id): Path<Uuid>,
    Query(q): Query<WarehouseQuery>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }
    match LocationRepo::list_for_warehouse(&state.pool, &q.tenant_id, warehouse_id).await {
        Ok(locs) => (StatusCode::OK, Json(json!(locs))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_location_routes_compile() {}
}

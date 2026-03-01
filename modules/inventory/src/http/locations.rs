//! Location HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/locations                  — create location
//!   GET   /api/inventory/locations/:id              — get location
//!   PUT   /api/inventory/locations/:id              — update location
//!   POST  /api/inventory/locations/:id/deactivate   — soft-delete location
//!   GET   /api/inventory/warehouses/:wid/locations  — list by warehouse
//!
//! Tenant identity derived from JWT `VerifiedClaims`.

use axum::{
    extract::{Path, State},
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
// Handlers
// ============================================================================

/// POST /api/inventory/locations
///
/// Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
pub async fn create_location(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateLocationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match LocationRepo::create(&state.pool, &req).await {
        Ok(loc) => (StatusCode::CREATED, Json(json!(loc))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// GET /api/inventory/locations/:id
///
/// Tenant derived from JWT VerifiedClaims.
pub async fn get_location(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match LocationRepo::find_by_id(&state.pool, id, &tenant_id).await {
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
///
/// Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
pub async fn update_location(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateLocationRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match LocationRepo::update(&state.pool, id, &req).await {
        Ok(loc) => (StatusCode::OK, Json(json!(loc))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// POST /api/inventory/locations/:id/deactivate
///
/// Tenant derived from JWT VerifiedClaims.
pub async fn deactivate_location(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match LocationRepo::deactivate(&state.pool, id, &tenant_id).await {
        Ok(loc) => (StatusCode::OK, Json(json!(loc))).into_response(),
        Err(e) => location_error_response(e).into_response(),
    }
}

/// GET /api/inventory/warehouses/:warehouse_id/locations
///
/// Tenant derived from JWT VerifiedClaims.
pub async fn list_locations(
    State(state): State<Arc<AppState>>,
    Path(warehouse_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match LocationRepo::list_for_warehouse(&state.pool, &tenant_id, warehouse_id).await {
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

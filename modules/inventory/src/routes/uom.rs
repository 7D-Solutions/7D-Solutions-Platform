//! UoM HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/uoms                             — create UoM
//!   GET  /api/inventory/uoms?tenant_id=...              — list UoMs for tenant
//!   POST /api/inventory/items/:id/uom-conversions       — add conversion
//!   GET  /api/inventory/items/:id/uom-conversions?...   — list conversions

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
    domain::uom::models::{
        ConversionRepo, CreateConversionRequest, CreateUomRequest, UomError, UomRepo,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn uom_error_response(err: UomError) -> impl IntoResponse {
    match err {
        UomError::DuplicateCode(code, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_code",
                "message": format!("UoM code '{}' already exists for tenant '{}'", code, tenant)
            })),
        )
            .into_response(),
        UomError::DuplicateConversion => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_conversion",
                "message": "Conversion already exists for this item and direction"
            })),
        )
            .into_response(),
        UomError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "UoM not found" })),
        )
            .into_response(),
        UomError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        UomError::Database(e) => {
            tracing::error!(error = %e, "uom database error");
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

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/uoms
///
/// Create a new unit of measure for the tenant.
/// Returns 201 Created with the UoM, 409 Conflict if code already exists,
/// 422 on validation failure.
pub async fn create_uom(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateUomRequest>,
) -> impl IntoResponse {
    match UomRepo::create(&state.pool, &req).await {
        Ok(uom) => (StatusCode::CREATED, Json(json!(uom))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

/// GET /api/inventory/uoms?tenant_id=...
///
/// List all UoMs for a tenant, ordered by code.
pub async fn list_uoms(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    match UomRepo::list_for_tenant(&state.pool, &q.tenant_id).await {
        Ok(uoms) => (StatusCode::OK, Json(json!(uoms))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

/// POST /api/inventory/items/:id/uom-conversions
///
/// Add a directional conversion factor for an item.
/// Returns 201 Created, 409 if direction already defined, 422 on validation failure.
pub async fn create_conversion(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<CreateConversionRequest>,
) -> impl IntoResponse {
    match ConversionRepo::create(&state.pool, item_id, &req).await {
        Ok(conv) => (StatusCode::CREATED, Json(json!(conv))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items/:id/uom-conversions?tenant_id=...
///
/// List all UoM conversions defined for an item.
pub async fn list_conversions(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    match ConversionRepo::list_for_item(&state.pool, item_id, &q.tenant_id).await {
        Ok(convs) => (StatusCode::OK, Json(json!(convs))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_uom_routes_compile() {}
}

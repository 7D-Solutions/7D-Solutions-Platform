//! UoM HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/uoms                             — create UoM
//!   GET  /api/inventory/uoms                             — list UoMs for tenant
//!   POST /api/inventory/items/:id/uom-conversions       — add conversion
//!   GET  /api/inventory/items/:id/uom-conversions       — list conversions
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
// Handlers
// ============================================================================

/// POST /api/inventory/uoms
///
/// Create a new unit of measure. Tenant derived from JWT VerifiedClaims — body tenant_id
/// is overridden.
/// Returns 201 Created with the UoM, 409 Conflict if code already exists,
/// 422 on validation failure.
pub async fn create_uom(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateUomRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match UomRepo::create(&state.pool, &req).await {
        Ok(uom) => (StatusCode::CREATED, Json(json!(uom))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

/// GET /api/inventory/uoms
///
/// List all UoMs for the tenant (from JWT), ordered by code.
pub async fn list_uoms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match UomRepo::list_for_tenant(&state.pool, &tenant_id).await {
        Ok(uoms) => (StatusCode::OK, Json(json!(uoms))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

/// POST /api/inventory/items/:id/uom-conversions
///
/// Add a directional conversion factor for an item. Tenant derived from JWT VerifiedClaims —
/// body tenant_id is overridden.
/// Returns 201 Created, 409 if direction already defined, 422 on validation failure.
pub async fn create_conversion(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Json(mut req): Json<CreateConversionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    match ConversionRepo::create(&state.pool, item_id, &req).await {
        Ok(conv) => (StatusCode::CREATED, Json(json!(conv))).into_response(),
        Err(e) => uom_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items/:id/uom-conversions
///
/// List all UoM conversions defined for an item. Tenant from JWT.
pub async fn list_conversions(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match ConversionRepo::list_for_item(&state.pool, item_id, &tenant_id).await {
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

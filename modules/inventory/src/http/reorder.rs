//! Reorder policy HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/reorder-policies              — create policy
//!   GET  /api/inventory/reorder-policies/:id          — get policy
//!   PUT  /api/inventory/reorder-policies/:id          — update policy
//!   GET  /api/inventory/items/:item_id/reorder-policies — list for item

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde_json::json;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::reorder::models::{
        CreateReorderPolicyRequest, ReorderPolicyError, ReorderPolicyRepo,
        UpdateReorderPolicyRequest,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn policy_error_response(err: ReorderPolicyError) -> impl IntoResponse {
    match err {
        ReorderPolicyError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Reorder policy not found" })),
        )
            .into_response(),

        ReorderPolicyError::DuplicatePolicy => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_policy",
                "message": "A reorder policy already exists for this item/location combination"
            })),
        )
            .into_response(),

        ReorderPolicyError::ItemNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "item_not_found",
                "message": "Item not found or does not belong to this tenant"
            })),
        )
            .into_response(),

        ReorderPolicyError::LocationNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "location_not_found",
                "message": "Location not found, inactive, or does not belong to this tenant"
            })),
        )
            .into_response(),

        ReorderPolicyError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),

        ReorderPolicyError::Database(e) => {
            tracing::error!(error = %e, "database error in reorder policy handler");
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

/// POST /api/inventory/reorder-policies
///
/// Creates a reorder policy for an item (optionally location-scoped).
/// Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
/// Returns 201 Created with the new policy.
pub async fn post_reorder_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateReorderPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    req.tenant_id = tenant_id;
    match ReorderPolicyRepo::create(&state.pool, &req).await {
        Ok(policy) => (StatusCode::CREATED, Json(json!(policy))).into_response(),
        Err(e) => policy_error_response(e).into_response(),
    }
}

/// GET /api/inventory/reorder-policies/:id
pub async fn get_reorder_policy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    match ReorderPolicyRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(policy)) => (StatusCode::OK, Json(json!(policy))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Reorder policy not found" })),
        )
            .into_response(),
        Err(e) => policy_error_response(e).into_response(),
    }
}

/// PUT /api/inventory/reorder-policies/:id
///
/// Updates threshold fields on an existing policy.
pub async fn put_reorder_policy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReorderPolicyRequest>,
) -> impl IntoResponse {
    match ReorderPolicyRepo::update(&state.pool, id, &req).await {
        Ok(policy) => (StatusCode::OK, Json(json!(policy))).into_response(),
        Err(e) => policy_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items/:item_id/reorder-policies
///
/// Lists all reorder policies for an item across all locations.
/// Tenant derived from JWT VerifiedClaims.
pub async fn list_reorder_policies(
    State(state): State<Arc<AppState>>,
    Path(item_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    match ReorderPolicyRepo::list_for_item(&state.pool, &tenant_id, item_id).await {
        Ok(policies) => (StatusCode::OK, Json(json!(policies))).into_response(),
        Err(e) => policy_error_response(e).into_response(),
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

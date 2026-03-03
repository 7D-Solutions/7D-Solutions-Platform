//! Lot genealogy HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/lots/split                   — split a lot
//!   POST /api/inventory/lots/merge                   — merge lots
//!   GET  /api/inventory/lots/{lot_id}/children       — forward trace
//!   GET  /api/inventory/lots/{lot_id}/parents        — reverse trace

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
    domain::genealogy::{
        children_of, parents_of, process_merge, process_split, GenealogyError, LotMergeRequest,
        LotSplitRequest,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn genealogy_error_response(err: GenealogyError) -> impl IntoResponse {
    match err {
        GenealogyError::LotNotFound(code) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": format!("Lot not found: {}", code) })),
        ),
        GenealogyError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        GenealogyError::QuantityConservation {
            children_sum,
            parent_qty,
        } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "quantity_conservation",
                "message": format!("children sum to {} but parent has {} on hand", children_sum, parent_qty),
            })),
        ),
        GenealogyError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "idempotency_conflict", "message": err.to_string() })),
        ),
        GenealogyError::Serialization(e) => {
            tracing::error!(error = %e, "genealogy serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        GenealogyError::Database(e) => {
            tracing::error!(error = %e, "genealogy database error");
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

/// POST /api/inventory/lots/split
pub async fn post_lot_split(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<LotSplitRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match process_split(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => genealogy_error_response(e).into_response(),
    }
}

/// POST /api/inventory/lots/merge
pub async fn post_lot_merge(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<LotMergeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match process_merge(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => genealogy_error_response(e).into_response(),
    }
}

/// GET /api/inventory/lots/{lot_id}/children
pub async fn get_lot_children(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(lot_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match children_of(&state.pool, &tenant_id, lot_id).await {
        Ok(edges) => (StatusCode::OK, Json(json!({ "edges": edges }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, lot_id = %lot_id, "database error querying children");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// GET /api/inventory/lots/{lot_id}/parents
pub async fn get_lot_parents(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(lot_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match parents_of(&state.pool, &tenant_id, lot_id).await {
        Ok(edges) => (StatusCode::OK, Json(json!({ "edges": edges }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, lot_id = %lot_id, "database error querying parents");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

//! Label generation HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items/:item_id/labels           — generate a label
//!   GET   /api/inventory/items/:item_id/labels           — list labels for item
//!   GET   /api/inventory/labels/:label_id                — get single label

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
    domain::labels::{generate_label, get_label, list_labels, GenerateLabelRequest, LabelError},
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn label_error_response(err: LabelError) -> impl IntoResponse {
    match err {
        LabelError::ItemNotFound | LabelError::RevisionNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": err.to_string() })),
        ),
        LabelError::ItemInactive => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "item_inactive", "message": err.to_string() })),
        ),
        LabelError::RevisionMismatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "revision_mismatch", "message": err.to_string() })),
        ),
        LabelError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "idempotency_conflict", "message": err.to_string() })),
        ),
        LabelError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        LabelError::Serialization(e) => {
            tracing::error!(error = %e, "label serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        LabelError::Database(e) => {
            tracing::error!(error = %e, "label database error");
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

/// POST /api/inventory/items/:item_id/labels
pub async fn post_generate_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Json(mut req): Json<GenerateLabelRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    req.item_id = item_id;

    // Propagate actor from claims if not explicitly set
    if req.actor_id.is_none() {
        if let Some(Extension(c)) = &claims {
            req.actor_id = Some(c.user_id);
        }
    }

    match generate_label(&state.pool, &req).await {
        Ok((label, false)) => (StatusCode::CREATED, Json(json!(label))).into_response(),
        Ok((label, true)) => (StatusCode::OK, Json(json!(label))).into_response(),
        Err(e) => label_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items/:item_id/labels
pub async fn get_list_labels(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match list_labels(&state.pool, &tenant_id, item_id).await {
        Ok(labels) => (StatusCode::OK, Json(json!(labels))).into_response(),
        Err(e) => label_error_response(e).into_response(),
    }
}

/// GET /api/inventory/labels/:label_id
pub async fn get_label_by_id(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(label_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match get_label(&state.pool, &tenant_id, label_id).await {
        Ok(Some(label)) => (StatusCode::OK, Json(json!(label))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Label not found" })),
        )
            .into_response(),
        Err(e) => label_error_response(e).into_response(),
    }
}

//! Work order parts HTTP handlers.
//!
//! Endpoints:
//!   POST   /api/maintenance/work-orders/:wo_id/parts           — add part
//!   GET    /api/maintenance/work-orders/:wo_id/parts           — list parts
//!   DELETE /api/maintenance/work-orders/:wo_id/parts/:part_id  — remove part

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

use crate::domain::work_orders::{AddPartRequest, WoPartError, WoPartsRepo};
use crate::AppState;

fn part_error_response(err: WoPartError) -> impl IntoResponse {
    match err {
        WoPartError::WoNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        ),
        WoPartError::PartNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Part not found" })),
        ),
        WoPartError::WoImmutable(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "wo_immutable",
                "message": format!("Cannot modify parts: work order status is {}", status)
            })),
        ),
        WoPartError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        WoPartError::Database(e) => {
            tracing::error!(error = %e, "work order parts database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/maintenance/work-orders/:wo_id/parts
pub async fn add_part(
    State(state): State<Arc<AppState>>,
    Path(wo_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<AddPartRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match WoPartsRepo::add(&state.pool, wo_id, &req).await {
        Ok(part) => (StatusCode::CREATED, Json(json!(part))).into_response(),
        Err(e) => part_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/work-orders/:wo_id/parts
pub async fn list_parts(
    State(state): State<Arc<AppState>>,
    Path(wo_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match WoPartsRepo::list(&state.pool, wo_id, &tenant_id).await {
        Ok(parts) => (StatusCode::OK, Json(json!(parts))).into_response(),
        Err(e) => part_error_response(e).into_response(),
    }
}

/// DELETE /api/maintenance/work-orders/:wo_id/parts/:part_id
pub async fn remove_part(
    State(state): State<Arc<AppState>>,
    Path((wo_id, part_id)): Path<(Uuid, Uuid)>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match WoPartsRepo::remove(&state.pool, wo_id, part_id, &tenant_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => part_error_response(e).into_response(),
    }
}

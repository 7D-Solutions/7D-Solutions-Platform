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
    domain::fg_receipt::{request_fg_receipt, FgReceiptError, RequestFgReceiptRequest},
    AppState,
};

/// POST /api/production/work-orders/:id/fg-receipt
pub async fn post_fg_receipt(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<RequestFgReceiptRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match request_fg_receipt(&state.pool, id, &req).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({ "status": "accepted", "work_order_id": id })),
        )
            .into_response(),
        Err(FgReceiptError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        )
            .into_response(),
        Err(FgReceiptError::NotReleased) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "not_released",
                "message": "Work order must be in 'released' status"
            })),
        )
            .into_response(),
        Err(FgReceiptError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        Err(FgReceiptError::Database(e)) => {
            tracing::error!(error = %e, "fg receipt database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

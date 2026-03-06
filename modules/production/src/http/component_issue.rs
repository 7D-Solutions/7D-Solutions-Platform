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
    domain::component_issue::{
        request_component_issue, ComponentIssueError, RequestComponentIssueRequest,
    },
    AppState,
};

/// POST /api/production/work-orders/:id/component-issues
pub async fn post_component_issue(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<RequestComponentIssueRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match request_component_issue(&state.pool, id, &req).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({ "status": "accepted", "work_order_id": id })),
        )
            .into_response(),
        Err(ComponentIssueError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        )
            .into_response(),
        Err(ComponentIssueError::NotReleased) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "not_released",
                "message": "Work order must be in 'released' status"
            })),
        )
            .into_response(),
        Err(ComponentIssueError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        Err(ComponentIssueError::Database(e)) => {
            tracing::error!(error = %e, "component issue database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

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
    domain::operations::{OperationError, OperationRepo},
    AppState,
};

fn operation_error_response(err: OperationError) -> impl IntoResponse {
    match err {
        OperationError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Operation not found" })),
        )
            .into_response(),
        OperationError::WorkOrderNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        )
            .into_response(),
        OperationError::WorkOrderNotReleased => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "work_order_not_released",
                "message": "Work order must be in 'released' status"
            })),
        )
            .into_response(),
        OperationError::NoRoutingTemplate => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_routing_template",
                "message": "Work order has no routing template assigned"
            })),
        )
            .into_response(),
        OperationError::AlreadyInitialized => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_initialized",
                "message": "Operations already initialized for this work order"
            })),
        )
            .into_response(),
        OperationError::InvalidTransition { from, to } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "invalid_transition",
                "message": format!("Cannot transition from '{}' to '{}'", from, to)
            })),
        )
            .into_response(),
        OperationError::PredecessorNotComplete(seq) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "predecessor_not_complete",
                "message": format!("Predecessor operation (seq {}) is not completed", seq)
            })),
        )
            .into_response(),
        OperationError::Database(e) => {
            tracing::error!(error = %e, "operation database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// POST /api/production/work-orders/:id/operations/initialize
pub async fn initialize_operations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(wo_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match OperationRepo::initialize(&state.pool, wo_id, &tenant_id, &corr, None).await {
        Ok(ops) => (StatusCode::CREATED, Json(json!(ops))).into_response(),
        Err(e) => operation_error_response(e).into_response(),
    }
}

/// POST /api/production/work-orders/:wo_id/operations/:op_id/start
pub async fn start_operation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((wo_id, op_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match OperationRepo::start(&state.pool, wo_id, op_id, &tenant_id, &corr, None).await {
        Ok(op) => (StatusCode::OK, Json(json!(op))).into_response(),
        Err(e) => operation_error_response(e).into_response(),
    }
}

/// POST /api/production/work-orders/:wo_id/operations/:op_id/complete
pub async fn complete_operation(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((wo_id, op_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match OperationRepo::complete(&state.pool, wo_id, op_id, &tenant_id, &corr, None).await {
        Ok(op) => (StatusCode::OK, Json(json!(op))).into_response(),
        Err(e) => operation_error_response(e).into_response(),
    }
}

/// GET /api/production/work-orders/:id/operations
pub async fn list_operations(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(wo_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match OperationRepo::list(&state.pool, wo_id, &tenant_id).await {
        Ok(ops) => (StatusCode::OK, Json(json!(ops))).into_response(),
        Err(e) => operation_error_response(e).into_response(),
    }
}

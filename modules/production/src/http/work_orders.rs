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
    domain::work_orders::{CreateWorkOrderRequest, WorkOrderError, WorkOrderRepo},
    AppState,
};

fn work_order_error_response(err: WorkOrderError) -> impl IntoResponse {
    match err {
        WorkOrderError::DuplicateOrderNumber(num, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_order_number",
                "message": format!(
                    "Order number '{}' already exists for tenant '{}'",
                    num, tenant
                )
            })),
        )
            .into_response(),
        WorkOrderError::DuplicateCorrelation => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_correlation",
                "message": "Work order with this correlation_id already exists"
            })),
        )
            .into_response(),
        WorkOrderError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        )
            .into_response(),
        WorkOrderError::InvalidTransition { from, to } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "invalid_transition",
                "message": format!("Cannot transition from '{}' to '{}'", from, to)
            })),
        )
            .into_response(),
        WorkOrderError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        WorkOrderError::Database(e) => {
            tracing::error!(error = %e, "work order database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// POST /api/production/work-orders
pub async fn create_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateWorkOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match WorkOrderRepo::create(&state.pool, &req, &corr, None).await {
        Ok(wo) => (StatusCode::CREATED, Json(json!(wo))).into_response(),
        Err(e) => work_order_error_response(e).into_response(),
    }
}

/// POST /api/production/work-orders/:id/release
pub async fn release_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match WorkOrderRepo::release(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(wo) => (StatusCode::OK, Json(json!(wo))).into_response(),
        Err(e) => work_order_error_response(e).into_response(),
    }
}

/// POST /api/production/work-orders/:id/close
pub async fn close_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match WorkOrderRepo::close(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(wo) => (StatusCode::OK, Json(json!(wo))).into_response(),
        Err(e) => work_order_error_response(e).into_response(),
    }
}

/// GET /api/production/work-orders/:id
pub async fn get_work_order(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match WorkOrderRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(wo)) => (StatusCode::OK, Json(json!(wo))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        )
            .into_response(),
        Err(e) => work_order_error_response(e).into_response(),
    }
}

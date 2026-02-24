//! Work order labor HTTP handlers.
//!
//! Endpoints:
//!   POST   /api/maintenance/work-orders/:wo_id/labor             — add labor
//!   GET    /api/maintenance/work-orders/:wo_id/labor             — list labor
//!   DELETE /api/maintenance/work-orders/:wo_id/labor/:labor_id   — remove labor

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::work_orders::{AddLaborRequest, WoLaborError, WoLaborRepo};
use crate::routes::work_orders::TenantQuery;
use crate::AppState;

fn labor_error_response(err: WoLaborError) -> impl IntoResponse {
    match err {
        WoLaborError::WoNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Work order not found" })),
        ),
        WoLaborError::LaborNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Labor entry not found" })),
        ),
        WoLaborError::WoImmutable(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "wo_immutable",
                "message": format!("Cannot modify labor: work order status is {}", status)
            })),
        ),
        WoLaborError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        WoLaborError::Database(e) => {
            tracing::error!(error = %e, "work order labor database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/maintenance/work-orders/:wo_id/labor
pub async fn add_labor(
    State(state): State<Arc<AppState>>,
    Path(wo_id): Path<Uuid>,
    Json(req): Json<AddLaborRequest>,
) -> impl IntoResponse {
    match WoLaborRepo::add(&state.pool, wo_id, &req).await {
        Ok(labor) => (StatusCode::CREATED, Json(json!(labor))).into_response(),
        Err(e) => labor_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/work-orders/:wo_id/labor
pub async fn list_labor(
    State(state): State<Arc<AppState>>,
    Path(wo_id): Path<Uuid>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    match WoLaborRepo::list(&state.pool, wo_id, &q.tenant_id).await {
        Ok(entries) => (StatusCode::OK, Json(json!(entries))).into_response(),
        Err(e) => labor_error_response(e).into_response(),
    }
}

/// DELETE /api/maintenance/work-orders/:wo_id/labor/:labor_id
pub async fn remove_labor(
    State(state): State<Arc<AppState>>,
    Path((wo_id, labor_id)): Path<(Uuid, Uuid)>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    match WoLaborRepo::remove(&state.pool, wo_id, labor_id, &q.tenant_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => labor_error_response(e).into_response(),
    }
}

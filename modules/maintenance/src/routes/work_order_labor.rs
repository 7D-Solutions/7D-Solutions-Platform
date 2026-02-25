//! Work order labor HTTP handlers.
//!
//! Endpoints:
//!   POST   /api/maintenance/work-orders/:wo_id/labor             — add labor
//!   GET    /api/maintenance/work-orders/:wo_id/labor             — list labor
//!   DELETE /api/maintenance/work-orders/:wo_id/labor/:labor_id   — remove labor

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

use crate::domain::work_orders::{AddLaborRequest, WoLaborError, WoLaborRepo};
use crate::AppState;
use super::ErrorBody;

fn labor_error_response(err: WoLaborError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        WoLaborError::WoNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Work order not found")),
        ),
        WoLaborError::LaborNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Labor entry not found")),
        ),
        WoLaborError::WoImmutable(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "wo_immutable",
                &format!("Cannot modify labor: work order status is {}", status),
            )),
        ),
        WoLaborError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        WoLaborError::Database(e) => {
            tracing::error!(error = %e, "work order labor database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/work-orders/:wo_id/labor
pub async fn add_labor(
    State(state): State<Arc<AppState>>,
    Path(wo_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<AddLaborRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match WoLaborRepo::add(&state.pool, wo_id, &req).await {
        Ok(labor) => (StatusCode::CREATED, Json(json!(labor))).into_response(),
        Err(e) => labor_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/work-orders/:wo_id/labor
pub async fn list_labor(
    State(state): State<Arc<AppState>>,
    Path(wo_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match WoLaborRepo::list(&state.pool, wo_id, &tenant_id).await {
        Ok(entries) => (StatusCode::OK, Json(json!(entries))).into_response(),
        Err(e) => labor_error_response(e).into_response(),
    }
}

/// DELETE /api/maintenance/work-orders/:wo_id/labor/:labor_id
pub async fn remove_labor(
    State(state): State<Arc<AppState>>,
    Path((wo_id, labor_id)): Path<(Uuid, Uuid)>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match crate::routes::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match WoLaborRepo::remove(&state.pool, wo_id, labor_id, &tenant_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => labor_error_response(e).into_response(),
    }
}

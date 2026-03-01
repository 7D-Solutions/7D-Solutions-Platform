//! Work order HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/maintenance/work-orders                — create work order
//!   GET   /api/maintenance/work-orders                — list work orders
//!   GET   /api/maintenance/work-orders/:id            — get work order
//!   PATCH /api/maintenance/work-orders/:id/transition — transition status

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::ErrorBody;
use crate::domain::work_orders::{
    CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WoError, WorkOrderRepo,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListWorkOrdersParams {
    pub asset_id: Option<Uuid>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn wo_error_response(err: WoError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        WoError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Work order not found")),
        ),
        WoError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Asset not found")),
        ),
        WoError::AssignmentNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Plan assignment not found")),
        ),
        WoError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        WoError::Transition(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("invalid_transition", &e.to_string())),
        ),
        WoError::Guard(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("guard_failed", &e.to_string())),
        ),
        WoError::Database(e) => {
            tracing::error!(error = %e, "work order database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/work-orders
pub async fn create_work_order(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateWorkOrderRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match WorkOrderRepo::create(&state.pool, &req).await {
        Ok(wo) => (StatusCode::CREATED, Json(json!(wo))).into_response(),
        Err(e) => wo_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/work-orders
pub async fn list_work_orders(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListWorkOrdersParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListWorkOrdersQuery {
        tenant_id,
        asset_id: params.asset_id,
        status: params.status,
        limit: params.limit,
        offset: params.offset,
    };
    match WorkOrderRepo::list(&state.pool, &q).await {
        Ok(orders) => (StatusCode::OK, Json(json!(orders))).into_response(),
        Err(e) => wo_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/work-orders/:id
pub async fn get_work_order(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match WorkOrderRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(wo)) => (StatusCode::OK, Json(json!(wo))).into_response(),
        Ok(None) => wo_error_response(WoError::NotFound).into_response(),
        Err(e) => wo_error_response(e).into_response(),
    }
}

/// PATCH /api/maintenance/work-orders/:id/transition
pub async fn transition_work_order(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<TransitionRequest>,
) -> impl IntoResponse {
    match WorkOrderRepo::transition(&state.pool, id, &req).await {
        Ok(wo) => (StatusCode::OK, Json(json!(wo))).into_response(),
        Err(e) => wo_error_response(e).into_response(),
    }
}

pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
    }
}

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
    domain::downtime::{DowntimeError, DowntimeRepo, EndDowntimeRequest, StartDowntimeRequest},
    AppState,
};

fn downtime_error_response(err: DowntimeError) -> impl IntoResponse {
    match err {
        DowntimeError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Downtime record not found" })),
        )
            .into_response(),
        DowntimeError::WorkcenterNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "workcenter_not_found", "message": "Workcenter not found" })),
        )
            .into_response(),
        DowntimeError::AlreadyEnded => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "already_ended", "message": "Downtime already ended" })),
        )
            .into_response(),
        DowntimeError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        DowntimeError::Database(e) => {
            tracing::error!(error = %e, "downtime database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// POST /api/production/workcenters/:id/downtime/start
pub async fn start_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(workcenter_id): Path<Uuid>,
    Json(mut req): Json<StartDowntimeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    req.workcenter_id = workcenter_id;
    let corr = Uuid::new_v4().to_string();
    match DowntimeRepo::start(&state.pool, &req, &corr, None).await {
        Ok(dt) => (StatusCode::CREATED, Json(json!(dt))).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

/// POST /api/production/downtime/:id/end
pub async fn end_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(downtime_id): Path<Uuid>,
    Json(mut req): Json<EndDowntimeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match DowntimeRepo::end(&state.pool, downtime_id, &req, &corr, None).await {
        Ok(dt) => (StatusCode::OK, Json(json!(dt))).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

/// GET /api/production/downtime/active
pub async fn list_active_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match DowntimeRepo::list_active(&state.pool, &tenant_id).await {
        Ok(list) => (StatusCode::OK, Json(json!(list))).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

/// GET /api/production/workcenters/:id/downtime
pub async fn list_workcenter_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(workcenter_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match DowntimeRepo::list_for_workcenter(&state.pool, workcenter_id, &tenant_id).await {
        Ok(list) => (StatusCode::OK, Json(json!(list))).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

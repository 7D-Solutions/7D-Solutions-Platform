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
    domain::workcenters::{
        CreateWorkcenterRequest, UpdateWorkcenterRequest, WorkcenterError, WorkcenterRepo,
    },
    AppState,
};

fn workcenter_error_response(err: WorkcenterError) -> impl IntoResponse {
    match err {
        WorkcenterError::DuplicateCode(code, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_code",
                "message": format!(
                    "Workcenter code '{}' already exists for tenant '{}'",
                    code, tenant
                )
            })),
        )
            .into_response(),
        WorkcenterError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Workcenter not found" })),
        )
            .into_response(),
        WorkcenterError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        WorkcenterError::Database(e) => {
            tracing::error!(error = %e, "workcenter database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// POST /api/production/workcenters
pub async fn create_workcenter(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateWorkcenterRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match WorkcenterRepo::create(&state.pool, &req, &corr, None).await {
        Ok(wc) => (StatusCode::CREATED, Json(json!(wc))).into_response(),
        Err(e) => workcenter_error_response(e).into_response(),
    }
}

/// GET /api/production/workcenters/:id
pub async fn get_workcenter(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match WorkcenterRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(wc)) => (StatusCode::OK, Json(json!(wc))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Workcenter not found" })),
        )
            .into_response(),
        Err(e) => workcenter_error_response(e).into_response(),
    }
}

/// GET /api/production/workcenters
pub async fn list_workcenters(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match WorkcenterRepo::list(&state.pool, &tenant_id).await {
        Ok(wcs) => (StatusCode::OK, Json(json!(wcs))).into_response(),
        Err(e) => workcenter_error_response(e).into_response(),
    }
}

/// PUT /api/production/workcenters/:id
pub async fn update_workcenter(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateWorkcenterRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match WorkcenterRepo::update(&state.pool, id, &req, &corr, None).await {
        Ok(wc) => (StatusCode::OK, Json(json!(wc))).into_response(),
        Err(e) => workcenter_error_response(e).into_response(),
    }
}

/// POST /api/production/workcenters/:id/deactivate
pub async fn deactivate_workcenter(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match WorkcenterRepo::deactivate(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(wc) => (StatusCode::OK, Json(json!(wc))).into_response(),
        Err(e) => workcenter_error_response(e).into_response(),
    }
}

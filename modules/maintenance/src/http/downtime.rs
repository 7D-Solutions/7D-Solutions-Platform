//! Downtime event HTTP handlers.
//!
//! Endpoints:
//!   POST /api/maintenance/downtime-events            — record downtime event
//!   GET  /api/maintenance/downtime-events            — list downtime events
//!   GET  /api/maintenance/downtime-events/:id        — get downtime event detail
//!   GET  /api/maintenance/assets/:asset_id/downtime  — list downtime for asset

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::ErrorBody;
use crate::domain::downtime::{
    CreateDowntimeRequest, DowntimeError, DowntimeRepo, ListDowntimeQuery,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListDowntimeParams {
    pub asset_id: Option<Uuid>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn downtime_error_response(err: DowntimeError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        DowntimeError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Downtime event not found")),
        ),
        DowntimeError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("asset_not_found", "Asset not found")),
        ),
        DowntimeError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        DowntimeError::IdempotentDuplicate(event) => (
            StatusCode::OK,
            Json(ErrorBody::new(
                "idempotent_duplicate",
                &format!("Downtime event {} already exists", event.id),
            )),
        ),
        DowntimeError::Database(e) => {
            tracing::error!(error = %e, "downtime database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/downtime-events
pub async fn create_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateDowntimeRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;

    match DowntimeRepo::create(&state.pool, &req).await {
        Ok(event) => (StatusCode::CREATED, Json(json!(event))).into_response(),
        Err(DowntimeError::IdempotentDuplicate(event)) => {
            (StatusCode::OK, Json(json!(event))).into_response()
        }
        Err(e) => downtime_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/downtime-events
pub async fn list_downtime(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListDowntimeParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListDowntimeQuery {
        tenant_id,
        asset_id: params.asset_id,
        from: params.from,
        to: params.to,
        limit: params.limit,
        offset: params.offset,
    };
    match DowntimeRepo::list(&state.pool, &q).await {
        Ok(events) => (StatusCode::OK, Json(json!(events))).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/downtime-events/:id
pub async fn get_downtime(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match DowntimeRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(event)) => (StatusCode::OK, Json(json!(event))).into_response(),
        Ok(None) => downtime_error_response(DowntimeError::NotFound).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets/:asset_id/downtime
pub async fn list_asset_downtime(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match DowntimeRepo::list_for_asset(&state.pool, asset_id, &tenant_id).await {
        Ok(events) => (StatusCode::OK, Json(json!(events))).into_response(),
        Err(e) => downtime_error_response(e).into_response(),
    }
}

fn extract_tenant(
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

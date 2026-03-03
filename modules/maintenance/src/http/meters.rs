//! Meter types and readings HTTP handlers.
//!
//! Endpoints:
//!   POST /api/maintenance/meter-types                 — create meter type
//!   GET  /api/maintenance/meter-types                 — list meter types
//!   POST /api/maintenance/assets/:id/readings         — record reading
//!   GET  /api/maintenance/assets/:id/readings         — list readings

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
use crate::domain::meters::{
    CreateMeterTypeRequest, ListReadingsQuery, MeterError, MeterReadingRepo, MeterTypeRepo,
    RecordReadingRequest,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListReadingsParams {
    pub meter_type_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn meter_error_response(err: MeterError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        MeterError::DuplicateName(name, tenant) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_meter_type",
                &format!(
                    "Meter type '{}' already exists for tenant '{}'",
                    name, tenant
                ),
            )),
        ),
        MeterError::MeterTypeNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Meter type not found")),
        ),
        MeterError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Asset not found")),
        ),
        MeterError::MonotonicityViolation {
            previous,
            attempted,
        } => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(
                "monotonicity_violation",
                &format!(
                    "Reading {} violates monotonicity: previous max was {}",
                    attempted, previous
                ),
            )),
        ),
        MeterError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        MeterError::Database(e) => {
            tracing::error!(error = %e, "meter database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

/// POST /api/maintenance/meter-types
pub async fn create_meter_type(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateMeterTypeRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::http::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match MeterTypeRepo::create(&state.pool, &req).await {
        Ok(mt) => (StatusCode::CREATED, Json(json!(mt))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/meter-types
pub async fn list_meter_types(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match crate::http::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    match MeterTypeRepo::list(&state.pool, &tenant_id).await {
        Ok(types) => (StatusCode::OK, Json(json!(types))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

/// POST /api/maintenance/assets/:id/readings
pub async fn record_reading(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<RecordReadingRequest>,
) -> impl IntoResponse {
    let tenant_id = match crate::http::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match MeterReadingRepo::record(&state.pool, asset_id, &req).await {
        Ok(reading) => (StatusCode::CREATED, Json(json!(reading))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets/:id/readings
pub async fn list_readings(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListReadingsParams>,
) -> impl IntoResponse {
    let tenant_id = match crate::http::work_orders::extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };

    let q = ListReadingsQuery {
        meter_type_id: params.meter_type_id,
        limit: params.limit,
        offset: params.offset,
    };

    match MeterReadingRepo::list(&state.pool, &tenant_id, asset_id, &q).await {
        Ok(readings) => (StatusCode::OK, Json(json!(readings))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

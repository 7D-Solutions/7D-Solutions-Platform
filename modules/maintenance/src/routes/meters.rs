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
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::meters::{
    CreateMeterTypeRequest, ListReadingsQuery, MeterError, MeterReadingRepo, MeterTypeRepo,
    RecordReadingRequest,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct TenantQuery {
    pub tenant_id: String,
}

fn meter_error_response(err: MeterError) -> impl IntoResponse {
    match err {
        MeterError::DuplicateName(name, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_meter_type",
                "message": format!("Meter type '{}' already exists for tenant '{}'", name, tenant)
            })),
        ),
        MeterError::MeterTypeNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Meter type not found" })),
        ),
        MeterError::AssetNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Asset not found" })),
        ),
        MeterError::MonotonicityViolation { previous, attempted } => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "monotonicity_violation",
                "message": format!(
                    "Reading {} violates monotonicity: previous max was {}",
                    attempted, previous
                ),
                "previous_max": previous,
                "attempted": attempted,
            })),
        ),
        MeterError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        MeterError::Database(e) => {
            tracing::error!(error = %e, "meter database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

/// POST /api/maintenance/meter-types
pub async fn create_meter_type(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateMeterTypeRequest>,
) -> impl IntoResponse {
    match MeterTypeRepo::create(&state.pool, &req).await {
        Ok(mt) => (StatusCode::CREATED, Json(json!(mt))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/meter-types
pub async fn list_meter_types(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    if q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match MeterTypeRepo::list(&state.pool, &q.tenant_id).await {
        Ok(types) => (StatusCode::OK, Json(json!(types))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

/// POST /api/maintenance/assets/:id/readings
pub async fn record_reading(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    Json(req): Json<RecordReadingRequest>,
) -> impl IntoResponse {
    match MeterReadingRepo::record(&state.pool, asset_id, &req).await {
        Ok(reading) => (StatusCode::CREATED, Json(json!(reading))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

/// GET /api/maintenance/assets/:id/readings
pub async fn list_readings(
    State(state): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    Query(tenant_q): Query<TenantQuery>,
    Query(q): Query<ListReadingsQuery>,
) -> impl IntoResponse {
    if tenant_q.tenant_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "validation_error", "message": "tenant_id is required" })),
        )
            .into_response();
    }

    match MeterReadingRepo::list(&state.pool, &tenant_q.tenant_id, asset_id, &q).await {
        Ok(readings) => (StatusCode::OK, Json(json!(readings))).into_response(),
        Err(e) => meter_error_response(e).into_response(),
    }
}

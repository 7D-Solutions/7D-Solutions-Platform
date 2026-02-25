//! Export run HTTP handlers — create, get, list.

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

use super::tenant::extract_tenant;
use crate::{
    domain::export::{
        models::{CreateExportRunRequest, ExportError},
        service,
    },
    AppState,
};

// ============================================================================
// Query params (without app_id — derived from JWT)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListExportsQuery {
    pub export_type: Option<String>,
}

// ============================================================================
// Error mapping
// ============================================================================

fn export_error_response(err: ExportError) -> impl IntoResponse {
    match err {
        ExportError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Export run not found" })),
        ),
        ExportError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ExportError::NoApprovedEntries => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_approved_entries",
                "message": "No approved timesheet entries found for this period"
            })),
        ),
        ExportError::IdempotentReplay { run_id } => (
            StatusCode::OK,
            Json(json!({
                "error": "idempotent_replay",
                "message": "Export with identical content already exists",
                "existing_run_id": run_id
            })),
        ),
        ExportError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/timekeeping/exports
pub async fn create_export(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateExportRunRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::create_export_run(&state.pool, &req).await {
        Ok(artifact) => (StatusCode::CREATED, Json(json!(artifact))).into_response(),
        Err(err) => export_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/exports/:id
pub async fn get_export(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::get_export_run(&state.pool, &app_id, id).await {
        Ok(run) => (StatusCode::OK, Json(json!(run))).into_response(),
        Err(err) => export_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/exports
pub async fn list_exports(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListExportsQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_export_runs(&state.pool, &app_id, q.export_type.as_deref()).await {
        Ok(runs) => (StatusCode::OK, Json(json!(runs))).into_response(),
        Err(err) => export_error_response(err).into_response(),
    }
}

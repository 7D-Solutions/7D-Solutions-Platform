//! Export run HTTP handlers — create, get, list.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::export::{
        models::{CreateExportRunRequest, ExportArtifact, ExportError, ExportRun},
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

fn map_export_error(err: ExportError) -> ApiError {
    match err {
        ExportError::NotFound => ApiError::not_found("Export run not found"),
        ExportError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        ExportError::NoApprovedEntries => ApiError::new(
            422,
            "no_approved_entries",
            "No approved timesheet entries found for this period",
        ),
        ExportError::IdempotentReplay { run_id } => ApiError::conflict(format!(
            "Export with identical content already exists: {}",
            run_id
        )),
        ExportError::Database(e) => ApiError::internal(e.to_string()),
    }
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/exports",
    request_body = CreateExportRunRequest,
    responses(
        (status = 201, description = "Export run created", body = ExportArtifact),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 409, description = "Idempotent replay", body = ApiError),
        (status = 422, description = "Validation or no approved entries", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Exports",
)]
pub async fn create_export(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateExportRunRequest>,
) -> Result<(StatusCode, Json<ExportArtifact>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let artifact = service::create_export_run(&state.pool, &req)
        .await
        .map_err(map_export_error)?;
    Ok((StatusCode::CREATED, Json(artifact)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/exports/{id}",
    params(("id" = Uuid, Path, description = "Export run UUID")),
    responses(
        (status = 200, description = "Export run found", body = ExportRun),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Exports",
)]
pub async fn get_export(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportRun>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let run = service::get_export_run(&state.pool, &app_id, id)
        .await
        .map_err(map_export_error)?;
    Ok(Json(run))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/exports",
    params(("export_type" = Option<String>, Query, description = "Filter by export type")),
    responses(
        (status = 200, description = "Export run list", body = Vec<ExportRun>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Exports",
)]
pub async fn list_exports(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListExportsQuery>,
) -> Result<Json<PaginatedResponse<ExportRun>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let runs = service::list_export_runs(&state.pool, &app_id, q.export_type.as_deref())
        .await
        .map_err(map_export_error)?;
    let total = runs.len() as i64;
    Ok(Json(PaginatedResponse::new(runs, 1, total, total)))
}

//! HTTP handlers for external refs CRUD and query endpoints.
//!
//! Routes:
//!   POST   /api/integrations/external-refs              — create/upsert
//!   GET    /api/integrations/external-refs/by-entity    — list by entity_type + entity_id
//!   GET    /api/integrations/external-refs/by-system    — lookup by system + external_id
//!   GET    /api/integrations/external-refs/:id          — get by id
//!   PUT    /api/integrations/external-refs/:id          — update label/metadata
//!   DELETE /api/integrations/external-refs/:id          — delete
//!
//! App identity derived from JWT `VerifiedClaims` (tenant/app scope).

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::external_refs::{
    service, CreateExternalRefRequest, ExternalRef, ExternalRefError, UpdateExternalRefRequest,
};
use crate::AppState;

// ============================================================================
// Helpers
// ============================================================================

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

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn ext_ref_error_response(e: ExternalRefError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        ExternalRefError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "not_found",
                &format!("External ref {} not found", id),
            )),
        ),
        ExternalRefError::Conflict(msg) => {
            (StatusCode::CONFLICT, Json(ErrorBody::new("conflict", &msg)))
        }
        ExternalRefError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        ExternalRefError::Database(e) => {
            tracing::error!("External ref DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

// ============================================================================
// Query param structs
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ByEntityQuery {
    pub entity_type: String,
    pub entity_id: String,
}

#[derive(Debug, Deserialize)]
pub struct BySystemQuery {
    pub system: String,
    pub external_id: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/integrations/external-refs — create or upsert an external ref
pub async fn create_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateExternalRefRequest>,
) -> Result<(StatusCode, Json<ExternalRef>), (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let created = service::create_external_ref(&state.pool, &app_id, &req, correlation_id)
        .await
        .map_err(ext_ref_error_response)?;

    Ok((StatusCode::CREATED, Json(created)))
}

/// GET /api/integrations/external-refs/by-entity — list refs by internal entity
pub async fn list_by_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ByEntityQuery>,
) -> Result<Json<Vec<ExternalRef>>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let refs = service::list_by_entity(&state.pool, &app_id, &q.entity_type, &q.entity_id)
        .await
        .map_err(ext_ref_error_response)?;

    Ok(Json(refs))
}

/// GET /api/integrations/external-refs/by-system — lookup ref by external system + id
pub async fn get_by_external(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<BySystemQuery>,
) -> Result<Json<ExternalRef>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let row = service::get_by_external(&state.pool, &app_id, &q.system, &q.external_id)
        .await
        .map_err(ext_ref_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "not_found",
                    &format!(
                        "No ref found for system={} external_id={}",
                        q.system, q.external_id
                    ),
                )),
            )
        })?;

    Ok(Json(row))
}

/// GET /api/integrations/external-refs/:id — get a single ref by id
pub async fn get_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(ref_id): Path<i64>,
) -> Result<Json<ExternalRef>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;

    let row = service::get_external_ref(&state.pool, &app_id, ref_id)
        .await
        .map_err(ext_ref_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "not_found",
                    &format!("External ref {} not found", ref_id),
                )),
            )
        })?;

    Ok(Json(row))
}

/// PUT /api/integrations/external-refs/:id — update label/metadata
pub async fn update_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(ref_id): Path<i64>,
    Json(req): Json<UpdateExternalRefRequest>,
) -> Result<Json<ExternalRef>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let updated = service::update_external_ref(&state.pool, &app_id, ref_id, &req, correlation_id)
        .await
        .map_err(ext_ref_error_response)?;

    Ok(Json(updated))
}

/// DELETE /api/integrations/external-refs/:id — hard delete
pub async fn delete_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(ref_id): Path<i64>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    service::delete_external_ref(&state.pool, &app_id, ref_id, correlation_id)
        .await
        .map_err(ext_ref_error_response)?;

    Ok(StatusCode::NO_CONTENT)
}

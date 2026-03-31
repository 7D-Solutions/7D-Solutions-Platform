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
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::external_refs::{
    service, CreateExternalRefRequest, ExternalRefError, UpdateExternalRefRequest,
};
use crate::AppState;

// ============================================================================
// Helpers
// ============================================================================

fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized("Missing or invalid authentication")),
    }
}

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn ext_ref_error(e: ExternalRefError) -> ApiError {
    match e {
        ExternalRefError::NotFound(id) => {
            ApiError::not_found(format!("External ref {} not found", id))
        }
        ExternalRefError::Conflict(msg) => ApiError::conflict(msg),
        ExternalRefError::Validation(msg) => {
            ApiError::new(422, "validation_error", msg)
        }
        ExternalRefError::Database(e) => {
            tracing::error!("External ref DB error: {}", e);
            ApiError::internal("Internal database error")
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
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::create_external_ref(&state.pool, &app_id, &req, correlation_id).await {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) => ext_ref_error(e).into_response(),
    }
}

/// GET /api/integrations/external-refs/by-entity — list refs by internal entity
pub async fn list_by_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ByEntityQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::list_by_entity(&state.pool, &app_id, &q.entity_type, &q.entity_id).await {
        Ok(refs) => {
            let total = refs.len() as i64;
            let resp = PaginatedResponse::new(refs, 1, total.max(1), total);
            Json(resp).into_response()
        }
        Err(e) => ext_ref_error(e).into_response(),
    }
}

/// GET /api/integrations/external-refs/by-system — lookup ref by external system + id
pub async fn get_by_external(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<BySystemQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_by_external(&state.pool, &app_id, &q.system, &q.external_id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => ApiError::not_found(format!(
            "No ref found for system={} external_id={}",
            q.system, q.external_id
        ))
        .into_response(),
        Err(e) => ext_ref_error(e).into_response(),
    }
}

/// GET /api/integrations/external-refs/:id — get a single ref by id
pub async fn get_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(ref_id): Path<i64>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_external_ref(&state.pool, &app_id, ref_id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => {
            ApiError::not_found(format!("External ref {} not found", ref_id)).into_response()
        }
        Err(e) => ext_ref_error(e).into_response(),
    }
}

/// PUT /api/integrations/external-refs/:id — update label/metadata
pub async fn update_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(ref_id): Path<i64>,
    Json(req): Json<UpdateExternalRefRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::update_external_ref(&state.pool, &app_id, ref_id, &req, correlation_id).await {
        Ok(updated) => Json(updated).into_response(),
        Err(e) => ext_ref_error(e).into_response(),
    }
}

/// DELETE /api/integrations/external-refs/:id — hard delete
pub async fn delete_external_ref(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(ref_id): Path<i64>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::delete_external_ref(&state.pool, &app_id, ref_id, correlation_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ext_ref_error(e).into_response(),
    }
}

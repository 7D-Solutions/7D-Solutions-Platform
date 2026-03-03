//! REST handlers for workforce competence API.
//!
//! Endpoints:
//!   POST /api/workforce-competence/artifacts       — register competence artifact
//!   GET  /api/workforce-competence/artifacts/{id}   — get artifact by ID
//!   POST /api/workforce-competence/assignments      — assign competence to operator
//!   GET  /api/workforce-competence/authorization    — check operator authorization

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
    domain::{
        acceptance_authority::{
            self, AcceptanceAuthorityQuery, GrantAuthorityRequest, RevokeAuthorityRequest,
        },
        guards::GuardError,
        models::{AssignCompetenceRequest, AuthorizationQuery, RegisterArtifactRequest},
        service::{self, ServiceError},
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn service_error_response(err: ServiceError) -> impl IntoResponse {
    match err {
        ServiceError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ServiceError::Guard(GuardError::ArtifactNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "artifact_not_found",
                "message": "Artifact not found or does not belong to this tenant"
            })),
        ),
        ServiceError::Guard(GuardError::ArtifactInactive) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "artifact_inactive",
                "message": "Artifact is inactive"
            })),
        ),
        ServiceError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
        ServiceError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "Idempotency key already used with a different request body"
            })),
        ),
        ServiceError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        ServiceError::Database(e) => {
            tracing::error!(error = %e, "database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/workforce-competence/artifacts
pub async fn post_artifact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<RegisterArtifactRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match service::register_artifact(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => service_error_response(e).into_response(),
    }
}

/// GET /api/workforce-competence/artifacts/{id}
pub async fn get_artifact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(artifact_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match service::get_artifact(&state.pool, &tenant_id, artifact_id).await {
        Ok(Some(artifact)) => (StatusCode::OK, Json(json!(artifact))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "artifact_not_found",
                "message": "Artifact not found or does not belong to this tenant"
            })),
        )
            .into_response(),
        Err(e) => service_error_response(e).into_response(),
    }
}

/// POST /api/workforce-competence/assignments
pub async fn post_assignment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<AssignCompetenceRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match service::assign_competence(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => service_error_response(e).into_response(),
    }
}

/// GET /api/workforce-competence/authorization
#[derive(Deserialize)]
pub struct AuthorizationQueryParams {
    pub operator_id: Uuid,
    pub artifact_code: String,
    pub at_time: chrono::DateTime<chrono::Utc>,
}

pub async fn get_authorization(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<AuthorizationQueryParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let query = AuthorizationQuery {
        tenant_id,
        operator_id: params.operator_id,
        artifact_code: params.artifact_code,
        at_time: params.at_time,
    };

    match service::check_authorization(&state.pool, &query).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => service_error_response(e).into_response(),
    }
}

// ============================================================================
// Acceptance authority handlers
// ============================================================================

/// POST /api/workforce-competence/acceptance-authorities
pub async fn post_grant_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<GrantAuthorityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match acceptance_authority::grant_acceptance_authority(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay { StatusCode::OK } else { StatusCode::CREATED };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => service_error_response(e).into_response(),
    }
}

/// POST /api/workforce-competence/acceptance-authorities/{id}/revoke
pub async fn post_revoke_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(authority_id): Path<Uuid>,
    Json(mut req): Json<RevokeAuthorityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    req.authority_id = authority_id;

    match acceptance_authority::revoke_acceptance_authority(&state.pool, &req).await {
        Ok((result, is_replay)) => {
            let status = if is_replay { StatusCode::OK } else { StatusCode::OK };
            (status, Json(json!(result))).into_response()
        }
        Err(e) => service_error_response(e).into_response(),
    }
}

/// GET /api/workforce-competence/acceptance-authority-check
#[derive(Deserialize)]
pub struct AcceptanceAuthorityCheckParams {
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub at_time: chrono::DateTime<chrono::Utc>,
}

pub async fn get_acceptance_authority_check(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<AcceptanceAuthorityCheckParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let query = AcceptanceAuthorityQuery {
        tenant_id,
        operator_id: params.operator_id,
        capability_scope: params.capability_scope,
        at_time: params.at_time,
    };

    match acceptance_authority::check_acceptance_authority(&state.pool, &query).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => service_error_response(e).into_response(),
    }
}

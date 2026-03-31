//! REST handlers for workforce competence API.
//!
//! Endpoints:
//!   POST /api/workforce-competence/artifacts              — register competence artifact
//!   GET  /api/workforce-competence/artifacts/{id}         — get artifact by ID
//!   POST /api/workforce-competence/assignments            — assign competence to operator
//!   GET  /api/workforce-competence/authorization          — check operator authorization
//!   POST /api/workforce-competence/acceptance-authorities — grant acceptance authority
//!   POST /api/workforce-competence/acceptance-authorities/{id}/revoke — revoke authority
//!   GET  /api/workforce-competence/acceptance-authority-check — check acceptance authority

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::{
    domain::{
        acceptance_authority::{
            self, AcceptanceAuthorityQuery, GrantAuthorityRequest, RevokeAuthorityRequest,
        },
        models::{AssignCompetenceRequest, AuthorizationQuery, RegisterArtifactRequest},
        service,
    },
    AppState,
};

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/workforce-competence/artifacts
pub async fn post_artifact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<RegisterArtifactRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) = service::register_artifact(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(result)))
}

/// GET /api/workforce-competence/artifacts/{id}
pub async fn get_artifact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(artifact_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let artifact = service::get_artifact(&state.pool, &tenant_id, artifact_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?
        .ok_or_else(|| {
            with_request_id(
                ApiError::not_found(
                    "Artifact not found or does not belong to this tenant",
                ),
                &ctx,
            )
        })?;

    Ok((StatusCode::OK, Json(artifact)))
}

/// POST /api/workforce-competence/assignments
pub async fn post_assignment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<AssignCompetenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) = service::assign_competence(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(result)))
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
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<AuthorizationQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let query = AuthorizationQuery {
        tenant_id,
        operator_id: params.operator_id,
        artifact_code: params.artifact_code,
        at_time: params.at_time,
    };

    let result = service::check_authorization(&state.pool, &query)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(result)))
}

// ============================================================================
// Acceptance authority handlers
// ============================================================================

/// POST /api/workforce-competence/acceptance-authorities
pub async fn post_grant_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<GrantAuthorityRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) =
        acceptance_authority::grant_acceptance_authority(&state.pool, &req)
            .await
            .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(result)))
}

/// POST /api/workforce-competence/acceptance-authorities/{id}/revoke
pub async fn post_revoke_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(authority_id): Path<Uuid>,
    Json(mut req): Json<RevokeAuthorityRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;
    req.authority_id = authority_id;

    let (result, _is_replay) =
        acceptance_authority::revoke_acceptance_authority(&state.pool, &req)
            .await
            .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(result)))
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
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<AcceptanceAuthorityCheckParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let query = AcceptanceAuthorityQuery {
        tenant_id,
        operator_id: params.operator_id,
        capability_scope: params.capability_scope,
        at_time: params.at_time,
    };

    let result =
        acceptance_authority::check_acceptance_authority(&state.pool, &query)
            .await
            .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(result)))
}

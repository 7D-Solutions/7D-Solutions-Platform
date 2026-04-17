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

use super::tenant::with_request_id;
use crate::{
    domain::{
        acceptance_authority::{
            self, AcceptanceAuthority, AcceptanceAuthorityQuery, AcceptanceAuthorityResult,
            GrantAuthorityRequest, RevokeAuthorityRequest,
        },
        models::{
            AssignCompetenceRequest, AuthorizationQuery, AuthorizationResult, CompetenceArtifact,
            OperatorCompetence, RegisterArtifactRequest,
        },
        service,
        training::{
            self as training_domain, CreateTrainingAssignmentRequest, CreateTrainingPlanRequest,
            RecordCompletionRequest, TrainingAssignment, TrainingCompletion, TrainingPlan,
            TransitionAssignmentRequest,
        },
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/workforce-competence/artifacts",
    tag = "Artifacts",
    request_body = RegisterArtifactRequest,
    responses(
        (status = 201, description = "Artifact created", body = CompetenceArtifact),
        (status = 200, description = "Idempotent replay", body = CompetenceArtifact),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_artifact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<RegisterArtifactRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
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

#[utoipa::path(
    get,
    path = "/api/workforce-competence/artifacts/{id}",
    tag = "Artifacts",
    params(("id" = Uuid, Path, description = "Artifact ID")),
    responses(
        (status = 200, description = "Artifact details", body = CompetenceArtifact),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn get_artifact(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(artifact_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let artifact = service::get_artifact(&state.pool, &tenant_id, artifact_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?
        .ok_or_else(|| {
            with_request_id(
                ApiError::not_found("Artifact not found or does not belong to this tenant"),
                &ctx,
            )
        })?;

    Ok((StatusCode::OK, Json(artifact)))
}

#[utoipa::path(
    post,
    path = "/api/workforce-competence/assignments",
    tag = "Assignments",
    request_body = AssignCompetenceRequest,
    responses(
        (status = 201, description = "Assignment created", body = OperatorCompetence),
        (status = 200, description = "Idempotent replay", body = OperatorCompetence),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_assignment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<AssignCompetenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
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
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuthorizationQueryParams {
    pub operator_id: Uuid,
    pub artifact_code: String,
    pub at_time: chrono::DateTime<chrono::Utc>,
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/authorization",
    tag = "Authorization",
    params(AuthorizationQueryParams),
    responses(
        (status = 200, description = "Authorization result", body = AuthorizationResult),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn get_authorization(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<AuthorizationQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

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

#[utoipa::path(
    post,
    path = "/api/workforce-competence/acceptance-authorities",
    tag = "Acceptance Authorities",
    request_body = GrantAuthorityRequest,
    responses(
        (status = 201, description = "Authority granted", body = AcceptanceAuthority),
        (status = 200, description = "Idempotent replay", body = AcceptanceAuthority),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_grant_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<GrantAuthorityRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) = acceptance_authority::grant_acceptance_authority(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(result)))
}

#[utoipa::path(
    post,
    path = "/api/workforce-competence/acceptance-authorities/{id}/revoke",
    tag = "Acceptance Authorities",
    params(("id" = Uuid, Path, description = "Authority ID")),
    request_body = RevokeAuthorityRequest,
    responses(
        (status = 200, description = "Authority revoked", body = AcceptanceAuthority),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_revoke_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(authority_id): Path<Uuid>,
    Json(mut req): Json<RevokeAuthorityRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;
    req.authority_id = authority_id;

    let (result, _is_replay) = acceptance_authority::revoke_acceptance_authority(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(result)))
}

/// GET /api/workforce-competence/acceptance-authority-check
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AcceptanceAuthorityCheckParams {
    pub operator_id: Uuid,
    pub capability_scope: String,
    pub at_time: chrono::DateTime<chrono::Utc>,
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/acceptance-authority-check",
    tag = "Acceptance Authorities",
    params(AcceptanceAuthorityCheckParams),
    responses(
        (status = 200, description = "Authority check result", body = AcceptanceAuthorityResult),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn get_acceptance_authority_check(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<AcceptanceAuthorityCheckParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let query = AcceptanceAuthorityQuery {
        tenant_id,
        operator_id: params.operator_id,
        capability_scope: params.capability_scope,
        at_time: params.at_time,
    };

    let result = acceptance_authority::check_acceptance_authority(&state.pool, &query)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(result)))
}

// ============================================================================
// Training delivery handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/workforce-competence/training-plans",
    tag = "Training",
    request_body = CreateTrainingPlanRequest,
    responses(
        (status = 201, description = "Training plan created", body = TrainingPlan),
        (status = 200, description = "Idempotent replay", body = TrainingPlan),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_training_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateTrainingPlanRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) = training_domain::create_training_plan(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(result)))
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/training-plans/{id}",
    tag = "Training",
    params(("id" = Uuid, Path, description = "Training plan ID")),
    responses(
        (status = 200, description = "Training plan", body = TrainingPlan),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn get_training_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(plan_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let plan = training_domain::get_training_plan(&state.pool, &tenant_id, plan_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?
        .ok_or_else(|| with_request_id(ApiError::not_found("Training plan not found"), &ctx))?;

    Ok((StatusCode::OK, Json(plan)))
}

/// GET /api/workforce-competence/training-plans
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListTrainingPlansParams {
    // No filters for now; lists all active plans for the tenant
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/training-plans",
    tag = "Training",
    responses(
        (status = 200, description = "List of training plans", body = Vec<TrainingPlan>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn list_training_plans(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let plans = training_domain::list_training_plans(&state.pool, &tenant_id)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(plans)))
}

#[utoipa::path(
    post,
    path = "/api/workforce-competence/training-assignments",
    tag = "Training",
    request_body = CreateTrainingAssignmentRequest,
    responses(
        (status = 201, description = "Assignment created", body = TrainingAssignment),
        (status = 200, description = "Idempotent replay", body = TrainingAssignment),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_training_assignment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateTrainingAssignmentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) = training_domain::create_training_assignment(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(result)))
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/training-assignments/{id}",
    tag = "Training",
    params(("id" = Uuid, Path, description = "Assignment ID")),
    responses(
        (status = 200, description = "Training assignment", body = TrainingAssignment),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn get_training_assignment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(assignment_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let assignment =
        training_domain::get_training_assignment(&state.pool, &tenant_id, assignment_id)
            .await
            .map_err(|e| with_request_id(ApiError::from(e), &ctx))?
            .ok_or_else(|| {
                with_request_id(ApiError::not_found("Training assignment not found"), &ctx)
            })?;

    Ok((StatusCode::OK, Json(assignment)))
}

/// PATCH /api/workforce-competence/training-assignments/{id}/status
#[utoipa::path(
    patch,
    path = "/api/workforce-competence/training-assignments/{id}/status",
    tag = "Training",
    params(("id" = Uuid, Path, description = "Assignment ID")),
    request_body = TransitionAssignmentRequest,
    responses(
        (status = 200, description = "Status updated", body = TrainingAssignment),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn patch_assignment_status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(assignment_id): Path<Uuid>,
    Json(mut req): Json<TransitionAssignmentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;
    req.assignment_id = assignment_id;

    let result = training_domain::transition_assignment_status(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(result)))
}

/// GET /api/workforce-competence/training-assignments
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListAssignmentsParams {
    pub plan_id: Option<Uuid>,
    pub operator_id: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/training-assignments",
    tag = "Training",
    params(ListAssignmentsParams),
    responses(
        (status = 200, description = "List of assignments", body = Vec<TrainingAssignment>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn list_training_assignments(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ListAssignmentsParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let assignments = training_domain::list_training_assignments(
        &state.pool,
        &tenant_id,
        params.plan_id,
        params.operator_id,
    )
    .await
    .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(assignments)))
}

#[utoipa::path(
    post,
    path = "/api/workforce-competence/training-completions",
    tag = "Training",
    request_body = RecordCompletionRequest,
    responses(
        (status = 201, description = "Completion recorded", body = TrainingCompletion),
        (status = 200, description = "Idempotent replay", body = TrainingCompletion),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_MUTATE"]))
)]
pub async fn post_training_completion(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<RecordCompletionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    req.tenant_id = tenant_id;

    let (result, is_replay) = training_domain::record_training_completion(&state.pool, &req)
        .await
        .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    let status = if is_replay { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(result)))
}

/// GET /api/workforce-competence/training-completions
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListCompletionsParams {
    pub plan_id: Option<Uuid>,
    pub operator_id: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/api/workforce-competence/training-completions",
    tag = "Training",
    params(ListCompletionsParams),
    responses(
        (status = 200, description = "List of completions", body = Vec<TrainingCompletion>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("bearer" = ["WORKFORCE_COMPETENCE_READ"]))
)]
pub async fn list_training_completions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ListCompletionsParams>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let completions = training_domain::list_training_completions(
        &state.pool,
        &tenant_id,
        params.plan_id,
        params.operator_id,
    )
    .await
    .map_err(|e| with_request_id(ApiError::from(e), &ctx))?;

    Ok((StatusCode::OK, Json(completions)))
}

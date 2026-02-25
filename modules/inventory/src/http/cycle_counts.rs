//! Cycle count task HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/cycle-count-tasks              — create a task + snapshot lines
//!   POST /api/inventory/cycle-count-tasks/{id}/submit  — submit counted quantities
//!
//! Full scope:   lines auto-populated from on-hand projection at the location.
//! Partial scope: lines built from caller-specified item_ids.
//!
//! Stock changes are NOT applied on submit. The approve endpoint (bd-opin) applies
//! adjustments after manager review.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::cycle_count::{
        approve_service::{approve_cycle_count, ApproveError, ApproveRequest},
        submit_service::{submit_cycle_count, SubmitError, SubmitLineInput, SubmitRequest},
        task_service::{create_cycle_count_task, CreateTaskRequest, TaskError},
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn task_error_response(err: TaskError) -> impl IntoResponse {
    match err {
        TaskError::MissingTenant => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "validation_error",
                "message": "tenant_id is required"
            })),
        )
            .into_response(),

        TaskError::EmptyPartialItemList => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "validation_error",
                "message": "partial scope requires at least one item_id"
            })),
        )
            .into_response(),

        TaskError::LocationNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "location_not_found",
                "message": "Location not found, inactive, or does not belong to this tenant/warehouse"
            })),
        )
            .into_response(),

        TaskError::Database(e) => {
            tracing::error!(error = %e, "database error creating cycle count task");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/inventory/cycle-count-tasks
///
/// Creates a cycle count task with snapshotted lines.
/// Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
/// Returns 201 Created with the full task including all lines.
pub async fn post_cycle_count_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    req.tenant_id = tenant_id;
    match create_cycle_count_task(&state.pool, &req).await {
        Ok(result) => (StatusCode::CREATED, Json(result)).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}

// ============================================================================
// Submit error mapping
// ============================================================================

fn submit_error_response(err: SubmitError) -> impl IntoResponse {
    match err {
        SubmitError::MissingTenant | SubmitError::MissingIdempotencyKey => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": err.to_string() })),
        )
            .into_response(),

        SubmitError::TaskNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "task_not_found", "message": err.to_string() })),
        )
            .into_response(),

        SubmitError::TaskNotOpen { .. } => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "task_not_open", "message": err.to_string() })),
        )
            .into_response(),

        SubmitError::LineNotFound { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "line_not_found", "message": err.to_string() })),
        )
            .into_response(),

        SubmitError::NegativeCountedQty { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": err.to_string() })),
        )
            .into_response(),

        SubmitError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "idempotency_conflict", "message": err.to_string() })),
        )
            .into_response(),

        SubmitError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in cycle count submit");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
                .into_response()
        }

        SubmitError::Database(e) => {
            tracing::error!(error = %e, "database error submitting cycle count");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Submit request body (task_id comes from URL path)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct SubmitBody {
    pub idempotency_key: String,
    #[serde(default)]
    pub lines: Vec<SubmitLineInput>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

// ============================================================================
// Submit handler
// ============================================================================

/// POST /api/inventory/cycle-count-tasks/{task_id}/submit
///
/// Submits counted quantities for an open cycle count task.
/// Tenant derived from JWT VerifiedClaims.
/// Returns 201 on first submit; 200 on idempotent replay.
pub async fn post_cycle_count_submit(
    Path(task_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<SubmitBody>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    let req = SubmitRequest {
        task_id,
        tenant_id,
        idempotency_key: body.idempotency_key,
        lines: body.lines,
        correlation_id: body.correlation_id,
        causation_id: body.causation_id,
    };
    match submit_cycle_count(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => submit_error_response(err).into_response(),
    }
}

// ============================================================================
// Approve error mapping
// ============================================================================

fn approve_error_response(err: ApproveError) -> impl IntoResponse {
    match err {
        ApproveError::MissingTenant | ApproveError::MissingIdempotencyKey => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": err.to_string() })),
        )
            .into_response(),

        ApproveError::TaskNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "task_not_found", "message": err.to_string() })),
        )
            .into_response(),

        ApproveError::TaskNotSubmitted { .. } => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "task_not_submitted", "message": err.to_string() })),
        )
            .into_response(),

        ApproveError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "idempotency_conflict", "message": err.to_string() })),
        )
            .into_response(),

        ApproveError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in cycle count approve");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
                .into_response()
        }

        ApproveError::Database(e) => {
            tracing::error!(error = %e, "database error approving cycle count");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Approve request body (task_id comes from URL path)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ApproveBody {
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

// ============================================================================
// Approve handler
// ============================================================================

/// POST /api/inventory/cycle-count-tasks/{task_id}/approve
///
/// Approves a submitted cycle count task, creating adjustment ledger entries
/// for all non-zero variances. Tenant derived from JWT VerifiedClaims.
/// Returns 201 on first approve; 200 on replay.
pub async fn post_cycle_count_approve(
    Path(task_id): Path<Uuid>,
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<ApproveBody>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    let req = ApproveRequest {
        task_id,
        tenant_id,
        idempotency_key: body.idempotency_key,
        correlation_id: body.correlation_id,
        causation_id: body.causation_id,
    };
    match approve_cycle_count(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => approve_error_response(err).into_response(),
    }
}

//! Period Close API Routes
//!
//! Provides HTTP endpoints for period close lifecycle operations:
//! - Validate Close: Pre-flight validation before closing
//! - Close Period: Atomically close a period with snapshot + hash
//! - Close Status: Query the current close status of a period

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use event_bus::TracingContext;
use chrono::{DateTime, Utc};
use platform_http_contracts::{ApiError, PaginatedResponse};
use serde::Serialize;
use security::VerifiedClaims;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::auth::with_request_id;
use crate::config::DEFAULT_REPORTING_CURRENCY;
use crate::contracts::period_close_v1::{
    ClosePeriodRequest, ClosePeriodResponse, CloseStatus, CloseStatusResponse,
    ValidateCloseRequest, ValidateCloseResponse,
};
use crate::services::period_close_service::{
    close_period, has_blocking_errors, validate_period_can_close, PeriodCloseError,
};
use crate::services::period_reopen_service;
use crate::AppState;

/// Map service errors to ApiError
fn map_error(error: PeriodCloseError) -> ApiError {
    match error {
        PeriodCloseError::PeriodNotFound(_) => ApiError::not_found(error.to_string()),
        PeriodCloseError::PeriodAlreadyClosed(_) => ApiError::conflict(error.to_string()),
        PeriodCloseError::ValidationFailed(_) => ApiError::bad_request(error.to_string()),
        PeriodCloseError::HashMismatch { .. } => ApiError::internal(error.to_string()),
        PeriodCloseError::Database(_) => ApiError::internal("Database error"),
        PeriodCloseError::FxRevaluation(_) => {
            ApiError::internal("FX revaluation failed during period close")
        }
    }
}

// ============================================================
// VALIDATE CLOSE ENDPOINT
// ============================================================

/// Handler for POST /api/gl/periods/{period_id}/validate-close
///
/// Pre-flight validation to check if a period can be closed.
/// Does NOT modify period state.
///
/// Returns validation report with errors/warnings.
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/validate-close", tag = "Period Close",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    request_body = ValidateCloseRequest,
    responses((status = 200, description = "Validation report", body = ValidateCloseResponse)),
    security(("bearer" = [])))]
pub async fn validate_close(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
    Json(_request): Json<ValidateCloseRequest>,
) -> Result<Json<ValidateCloseResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    // Run validation in a transaction (read-only, but ensures consistency)
    let mut tx = app_state
        .pool
        .begin()
        .await
        .map_err(|e| with_request_id(ApiError::internal(format!("Failed to begin transaction: {}", e)), &ctx))?;

    let validation_report = validate_period_can_close(
        &mut tx,
        &tenant_id,
        period_id,
        app_state.dlq_validation_enabled,
    )
    .await
    .map_err(|e| with_request_id(map_error(e), &ctx))?;

    tx.commit().await.map_err(|e| with_request_id(ApiError::internal(format!("Failed to commit transaction: {}", e)), &ctx))?;

    let can_close = !has_blocking_errors(&validation_report);

    Ok(Json(ValidateCloseResponse {
        period_id,
        tenant_id,
        can_close,
        validation_report,
        validated_at: chrono::Utc::now(),
    }))
}

// ============================================================
// CLOSE PERIOD ENDPOINT
// ============================================================

/// Handler for POST /api/gl/periods/{period_id}/close
///
/// Atomically close an accounting period.
/// Idempotent - if already closed, returns existing close status.
///
/// Steps:
/// 1. Lock period row (FOR UPDATE)
/// 2. Check if already closed (idempotency)
/// 3. Run pre-close validation
/// 4. Create sealed snapshot with hash
/// 5. Update period with close fields
///
/// Returns close status on success, validation report on failure.
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/close", tag = "Period Close",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    request_body = ClosePeriodRequest,
    responses((status = 200, description = "Period closed", body = ClosePeriodResponse)),
    security(("bearer" = [])))]
pub async fn close_period_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<ClosePeriodRequest>,
) -> Result<Json<ClosePeriodResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let result = close_period(
        &app_state.pool,
        &tenant_id,
        period_id,
        &request.closed_by,
        request.close_reason.as_deref(),
        app_state.dlq_validation_enabled,
        DEFAULT_REPORTING_CURRENCY,
    )
    .await
    .map_err(|e| with_request_id(map_error(e), &ctx))?;

    Ok(Json(ClosePeriodResponse {
        period_id: result.period_id,
        tenant_id: result.tenant_id,
        success: result.success,
        close_status: result.close_status,
        validation_report: result.validation_report,
        timestamp: result.timestamp,
    }))
}

// ============================================================
// CLOSE STATUS ENDPOINT
// ============================================================

/// Period data for close status query
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct PeriodCloseStatusData {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub closed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub closed_by: Option<String>,
    pub close_reason: Option<String>,
    pub close_hash: Option<String>,
    pub close_requested_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Handler for GET /api/gl/periods/{period_id}/close-status?tenant_id={tenant_id}
///
/// Query the current close status of a period.
/// Returns period date range and close lifecycle state.
///
/// O(1) query - single row lookup, no unbounded reads.
#[utoipa::path(get, path = "/api/gl/periods/{period_id}/close-status", tag = "Period Close",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    responses((status = 200, description = "Close status", body = CloseStatusResponse)),
    security(("bearer" = [])))]
pub async fn get_close_status(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<CloseStatusResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    // Single-row query (O(1) per period)
    let period = sqlx::query_as::<_, PeriodCloseStatusData>(
        r#"
        SELECT id, tenant_id, period_start, period_end,
               closed_at, closed_by, close_reason, close_hash, close_requested_at
        FROM accounting_periods
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(period_id)
    .bind(&tenant_id)
    .fetch_optional(&app_state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?
    .ok_or_else(|| {
        with_request_id(
            ApiError::not_found(format!("Period {} not found for tenant {}", period_id, tenant_id)),
            &ctx,
        )
    })?;

    // Determine close status from period fields
    let close_status = if let Some(closed_at) = period.closed_at {
        CloseStatus::Closed {
            closed_at,
            closed_by: period.closed_by.unwrap_or_default(),
            close_reason: period.close_reason,
            close_hash: period.close_hash.unwrap_or_default(),
            requested_at: period.close_requested_at,
        }
    } else if let Some(requested_at) = period.close_requested_at {
        CloseStatus::CloseRequested { requested_at }
    } else {
        CloseStatus::Open
    };

    Ok(Json(CloseStatusResponse {
        period_id,
        tenant_id,
        period_start: period.period_start.to_string(),
        period_end: period.period_end.to_string(),
        close_status,
        timestamp: chrono::Utc::now(),
    }))
}

// ============================================================
// REOPEN ENDPOINTS (Phase 31, bd-2rl9)
// ============================================================

/// Reopen request payload
#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ReopenRequestPayload {
    pub requested_by: String,
    pub reason: String,
}

/// Reopen approve payload
#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ReopenApprovePayload {
    pub approved_by: String,
}

/// Reopen reject payload
#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ReopenRejectPayload {
    pub rejected_by: String,
    pub reject_reason: String,
}

/// Response type for reopen request list entries
#[derive(Debug, Serialize, ToSchema)]
pub struct ReopenRequestResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub requested_by: String,
    pub reason: String,
    pub prior_close_hash: String,
    pub status: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub rejected_by: Option<String>,
    pub rejected_at: Option<DateTime<Utc>>,
    pub reject_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// POST /api/gl/periods/{period_id}/reopen — request a controlled reopen
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/reopen", tag = "Period Close",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    request_body = ReopenRequestPayload,
    responses((status = 201, description = "Reopen request created", body = serde_json::Value)),
    security(("bearer" = [])))]
pub async fn request_reopen(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<ReopenRequestPayload>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let result = period_reopen_service::request_reopen(
        &app_state.pool,
        &tenant_id,
        period_id,
        &request.requested_by,
        &request.reason,
    )
    .await
    .map_err(|e| with_request_id(map_error(e), &ctx))?;

    let json_val = serde_json::to_value(result)
        .map_err(|e| with_request_id(ApiError::internal(format!("serialization error: {e}")), &ctx))?;
    Ok((StatusCode::CREATED, Json(json_val)))
}

/// POST /api/gl/periods/{period_id}/reopen/{request_id}/approve
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/reopen/{request_id}/approve", tag = "Period Close",
    params(
        ("period_id" = Uuid, Path, description = "Accounting period ID"),
        ("request_id" = Uuid, Path, description = "Reopen request ID"),
    ),
    request_body = ReopenApprovePayload,
    responses((status = 200, description = "Reopen approved", body = serde_json::Value)),
    security(("bearer" = [])))]
pub async fn approve_reopen(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path((period_id, request_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ReopenApprovePayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let result = period_reopen_service::approve_reopen(
        &app_state.pool,
        &tenant_id,
        period_id,
        request_id,
        &request.approved_by,
    )
    .await
    .map_err(|e| with_request_id(map_error(e), &ctx))?;

    let json_val = serde_json::to_value(result)
        .map_err(|e| with_request_id(ApiError::internal(format!("serialization error: {e}")), &ctx))?;
    Ok(Json(json_val))
}

/// POST /api/gl/periods/{period_id}/reopen/{request_id}/reject
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/reopen/{request_id}/reject", tag = "Period Close",
    params(
        ("period_id" = Uuid, Path, description = "Accounting period ID"),
        ("request_id" = Uuid, Path, description = "Reopen request ID"),
    ),
    request_body = ReopenRejectPayload,
    responses((status = 200, description = "Reopen rejected", body = serde_json::Value)),
    security(("bearer" = [])))]
pub async fn reject_reopen(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path((period_id, request_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ReopenRejectPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let result = period_reopen_service::reject_reopen(
        &app_state.pool,
        &tenant_id,
        period_id,
        request_id,
        &request.rejected_by,
        &request.reject_reason,
    )
    .await
    .map_err(|e| with_request_id(map_error(e), &ctx))?;

    let json_val = serde_json::to_value(result)
        .map_err(|e| with_request_id(ApiError::internal(format!("serialization error: {e}")), &ctx))?;
    Ok(Json(json_val))
}

/// GET /api/gl/periods/{period_id}/reopen
#[utoipa::path(get, path = "/api/gl/periods/{period_id}/reopen", tag = "Period Close",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    responses((status = 200, description = "List of reopen requests", body = PaginatedResponse<ReopenRequestResponse>)),
    security(("bearer" = [])))]
pub async fn list_reopen_requests(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<PaginatedResponse<ReopenRequestResponse>>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let rows = period_reopen_service::list_reopen_requests(&app_state.pool, &tenant_id, period_id)
        .await
        .map_err(|e| with_request_id(map_error(e), &ctx))?;

    let items: Vec<ReopenRequestResponse> = rows
        .into_iter()
        .map(|r| ReopenRequestResponse {
            id: r.id,
            tenant_id: r.tenant_id,
            period_id: r.period_id,
            requested_by: r.requested_by,
            reason: r.reason,
            prior_close_hash: r.prior_close_hash,
            status: r.status,
            approved_by: r.approved_by,
            approved_at: r.approved_at,
            rejected_by: r.rejected_by,
            rejected_at: r.rejected_at,
            reject_reason: r.reject_reason,
            created_at: r.created_at,
        })
        .collect();
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse::new(items, 1, total, total)))
}

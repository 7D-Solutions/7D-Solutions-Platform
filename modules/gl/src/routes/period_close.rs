//! Period Close API Routes
//!
//! Provides HTTP endpoints for period close lifecycle operations:
//! - Validate Close: Pre-flight validation before closing
//! - Close Period: Atomically close a period with snapshot + hash
//! - Close Status: Query the current close status of a period

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;
use super::auth::extract_tenant;
use crate::config::DEFAULT_REPORTING_CURRENCY;
use crate::contracts::period_close_v1::{
    CloseStatus, ClosePeriodRequest, ClosePeriodResponse,
    CloseStatusResponse, ValidateCloseRequest, ValidateCloseResponse,
};
use crate::services::period_close_service::{
    close_period, has_blocking_errors, validate_period_can_close, PeriodCloseError,
};
use crate::services::period_reopen_service;

/// Error response wrapper
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Period close error HTTP response
#[derive(Debug)]
pub struct PeriodCloseHttpError {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for PeriodCloseHttpError {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

/// Map service errors to HTTP status codes
fn map_error(error: PeriodCloseError) -> PeriodCloseHttpError {
    match error {
        PeriodCloseError::PeriodNotFound(_) => PeriodCloseHttpError {
            status: StatusCode::NOT_FOUND,
            message: error.to_string(),
        },
        PeriodCloseError::PeriodAlreadyClosed(_) => PeriodCloseHttpError {
            status: StatusCode::CONFLICT,
            message: error.to_string(),
        },
        PeriodCloseError::ValidationFailed(_) => PeriodCloseHttpError {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        },
        PeriodCloseError::HashMismatch { .. } => PeriodCloseHttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        },
        PeriodCloseError::Database(_) => PeriodCloseHttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Database error".to_string(), // Don't leak internal details
        },
        PeriodCloseError::FxRevaluation(_) => PeriodCloseHttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "FX revaluation failed during period close".to_string(),
        },
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
pub async fn validate_close(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(period_id): Path<Uuid>,
    Json(_request): Json<ValidateCloseRequest>,
) -> Result<Json<ValidateCloseResponse>, PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    // Run validation in a transaction (read-only, but ensures consistency)
    let mut tx = app_state.pool.begin().await.map_err(|e| PeriodCloseHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to begin transaction: {}", e),
    })?;

    let validation_report =
        validate_period_can_close(&mut tx, &tenant_id, period_id, app_state.dlq_validation_enabled)
            .await
            .map_err(map_error)?;

    tx.commit().await.map_err(|e| PeriodCloseHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to commit transaction: {}", e),
    })?;

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
pub async fn close_period_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<ClosePeriodRequest>,
) -> Result<Json<ClosePeriodResponse>, PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

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
    .map_err(map_error)?;

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
pub async fn get_close_status(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<CloseStatusResponse>, PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

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
    .map_err(|e| PeriodCloseHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Database error: {}", e),
    })?
    .ok_or_else(|| PeriodCloseHttpError {
        status: StatusCode::NOT_FOUND,
        message: format!(
            "Period {} not found for tenant {}",
            period_id, tenant_id
        ),
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
        CloseStatus::CloseRequested {
            requested_at,
        }
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
#[derive(Debug, serde::Deserialize)]
pub struct ReopenRequestPayload {
    pub requested_by: String,
    pub reason: String,
}

/// Reopen approve payload
#[derive(Debug, serde::Deserialize)]
pub struct ReopenApprovePayload {
    pub approved_by: String,
}

/// Reopen reject payload
#[derive(Debug, serde::Deserialize)]
pub struct ReopenRejectPayload {
    pub rejected_by: String,
    pub reject_reason: String,
}

/// POST /api/gl/periods/{period_id}/reopen — request a controlled reopen
pub async fn request_reopen(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<ReopenRequestPayload>,
) -> Result<(StatusCode, Json<serde_json::Value>), PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let result = period_reopen_service::request_reopen(
        &app_state.pool,
        &tenant_id,
        period_id,
        &request.requested_by,
        &request.reason,
    )
    .await
    .map_err(map_error)?;

    Ok((StatusCode::CREATED, Json(serde_json::to_value(result).unwrap())))
}

/// POST /api/gl/periods/{period_id}/reopen/{request_id}/approve
pub async fn approve_reopen(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((period_id, request_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ReopenApprovePayload>,
) -> Result<Json<serde_json::Value>, PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let result = period_reopen_service::approve_reopen(
        &app_state.pool,
        &tenant_id,
        period_id,
        request_id,
        &request.approved_by,
    )
    .await
    .map_err(map_error)?;

    Ok(Json(serde_json::to_value(result).unwrap()))
}

/// POST /api/gl/periods/{period_id}/reopen/{request_id}/reject
pub async fn reject_reopen(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((period_id, request_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ReopenRejectPayload>,
) -> Result<Json<serde_json::Value>, PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let result = period_reopen_service::reject_reopen(
        &app_state.pool,
        &tenant_id,
        period_id,
        request_id,
        &request.rejected_by,
        &request.reject_reason,
    )
    .await
    .map_err(map_error)?;

    Ok(Json(serde_json::to_value(result).unwrap()))
}

/// GET /api/gl/periods/{period_id}/reopen
pub async fn list_reopen_requests(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, PeriodCloseHttpError> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| PeriodCloseHttpError {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    let rows = period_reopen_service::list_reopen_requests(
        &app_state.pool,
        &tenant_id,
        period_id,
    )
    .await
    .map_err(map_error)?;

    Ok(Json(serde_json::to_value(rows).unwrap()))
}

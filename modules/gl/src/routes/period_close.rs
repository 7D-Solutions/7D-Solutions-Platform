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
    Json,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::contracts::period_close_v1::{
    CloseStatus, ClosePeriodRequest, ClosePeriodResponse, CloseStatusRequest,
    CloseStatusResponse, ValidateCloseRequest, ValidateCloseResponse,
};
use crate::services::period_close_service::{
    close_period, has_blocking_errors, validate_period_can_close, PeriodCloseError,
};

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
    State(pool): State<Arc<PgPool>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<ValidateCloseRequest>,
) -> Result<Json<ValidateCloseResponse>, PeriodCloseHttpError> {
    // Run validation in a transaction (read-only, but ensures consistency)
    let mut tx = pool.begin().await.map_err(|e| PeriodCloseHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to begin transaction: {}", e),
    })?;

    let validation_report =
        validate_period_can_close(&mut tx, &request.tenant_id, period_id)
            .await
            .map_err(map_error)?;

    tx.commit().await.map_err(|e| PeriodCloseHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to commit transaction: {}", e),
    })?;

    let can_close = !has_blocking_errors(&validation_report);

    Ok(Json(ValidateCloseResponse {
        period_id,
        tenant_id: request.tenant_id,
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
    State(pool): State<Arc<PgPool>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<ClosePeriodRequest>,
) -> Result<Json<ClosePeriodResponse>, PeriodCloseHttpError> {
    let result = close_period(
        &pool,
        &request.tenant_id,
        period_id,
        &request.closed_by,
        request.close_reason.as_deref(),
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
    State(pool): State<Arc<PgPool>>,
    Path(period_id): Path<Uuid>,
    axum::extract::Query(request): axum::extract::Query<CloseStatusRequest>,
) -> Result<Json<CloseStatusResponse>, PeriodCloseHttpError> {
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
    .bind(&request.tenant_id)
    .fetch_optional(pool.as_ref())
    .await
    .map_err(|e| PeriodCloseHttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Database error: {}", e),
    })?
    .ok_or_else(|| PeriodCloseHttpError {
        status: StatusCode::NOT_FOUND,
        message: format!(
            "Period {} not found for tenant {}",
            period_id, request.tenant_id
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
    } else if period.close_requested_at.is_some() {
        CloseStatus::CloseRequested {
            requested_at: period.close_requested_at.unwrap(),
        }
    } else {
        CloseStatus::Open
    };

    Ok(Json(CloseStatusResponse {
        period_id,
        tenant_id: request.tenant_id,
        period_start: period.period_start.to_string(),
        period_end: period.period_end.to_string(),
        close_status,
        timestamp: chrono::Utc::now(),
    }))
}

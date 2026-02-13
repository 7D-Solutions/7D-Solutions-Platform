//! Period Summary API Routes
//!
//! Provides HTTP endpoints for querying period summary reports.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::period_summary_service::{self, PeriodSummaryResponse};

/// Query parameters for period summary endpoint
#[derive(Debug, Deserialize)]
pub struct PeriodSummaryQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Optional currency filter (e.g., "USD", "EUR")
    pub currency: Option<String>,
}

/// Error response
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/gl/periods/{period_id}/summary
///
/// Returns period summary for a tenant and period with optional currency filter.
/// Prefers precomputed snapshot if present, otherwise computes from account_balances.
pub async fn get_period_summary(
    State(pool): State<Arc<PgPool>>,
    Path(period_id): Path<Uuid>,
    Query(params): Query<PeriodSummaryQuery>,
) -> Result<Json<PeriodSummaryResponse>, PeriodSummaryErrorResponse> {
    // Query period summary (service layer handles snapshot vs computed logic)
    let response = period_summary_service::get_period_summary(
        &pool,
        &params.tenant_id,
        period_id,
        params.currency.as_deref(),
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            period_summary_service::PeriodSummaryServiceError::InvalidTenantId(_) => {
                StatusCode::BAD_REQUEST
            }
            period_summary_service::PeriodSummaryServiceError::InvalidCurrency(_) => {
                StatusCode::BAD_REQUEST
            }
            period_summary_service::PeriodSummaryServiceError::Repo(ref repo_err) => {
                match repo_err {
                    crate::repos::period_summary_repo::PeriodSummaryError::PeriodNotFound(_) => {
                        StatusCode::NOT_FOUND
                    }
                    crate::repos::period_summary_repo::PeriodSummaryError::Database(_) => {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }
            }
        };

        PeriodSummaryErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct PeriodSummaryErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for PeriodSummaryErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

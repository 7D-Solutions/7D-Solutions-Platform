//! Trial Balance API Routes
//!
//! Provides HTTP endpoints for querying trial balance reports.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::trial_balance_service::{self, TrialBalanceResponse};

/// Query parameters for trial balance endpoint
#[derive(Debug, Deserialize)]
pub struct TrialBalanceQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Accounting period ID
    pub period_id: Uuid,
    /// Optional currency filter (e.g., "USD", "EUR")
    pub currency: Option<String>,
}

/// Trial balance error response
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/gl/trial-balance
///
/// Returns trial balance for a tenant and period with optional currency filter.
pub async fn get_trial_balance(
    State(pool): State<Arc<PgPool>>,
    Query(params): Query<TrialBalanceQuery>,
) -> Result<Json<TrialBalanceResponse>, TrialBalanceErrorResponse> {
    // Query trial balance (service layer handles all transformation and totals calculation)
    let response = trial_balance_service::get_trial_balance(
        &pool,
        &params.tenant_id,
        params.period_id,
        params.currency.as_deref(),
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            trial_balance_service::TrialBalanceError::InvalidTenantId(_) => StatusCode::BAD_REQUEST,
            trial_balance_service::TrialBalanceError::InvalidCurrency(_) => StatusCode::BAD_REQUEST,
            trial_balance_service::TrialBalanceError::BalanceRepo(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        TrialBalanceErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct TrialBalanceErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for TrialBalanceErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

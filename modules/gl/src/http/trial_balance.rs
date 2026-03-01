//! Trial Balance API Routes
//!
//! Provides HTTP endpoints for querying trial balance reports.

use crate::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::auth::extract_tenant;
use crate::services::trial_balance_service::{self, TrialBalanceResponse};

/// Query parameters for trial balance endpoint
#[derive(Debug, Deserialize)]
pub struct TrialBalanceQuery {
    /// Accounting period ID
    pub period_id: Uuid,
    /// Currency code (ISO 4217, optional) - e.g., "USD", "EUR". If omitted, all currencies are returned.
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
/// Tenant identity is derived from JWT claims (VerifiedClaims).
pub async fn get_trial_balance(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<TrialBalanceQuery>,
) -> Result<Json<TrialBalanceResponse>, TrialBalanceErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| TrialBalanceErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    // Query trial balance (service layer handles all transformation and totals calculation)
    let response = trial_balance_service::get_trial_balance(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        params.currency.as_deref().unwrap_or("USD"),
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            trial_balance_service::TrialBalanceError::InvalidTenantId(_) => StatusCode::BAD_REQUEST,
            trial_balance_service::TrialBalanceError::Unbalanced { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            trial_balance_service::TrialBalanceError::StatementRepo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
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

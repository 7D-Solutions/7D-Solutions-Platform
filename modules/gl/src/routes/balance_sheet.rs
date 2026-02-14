//! Balance Sheet API Routes (Phase 14.7)
//!
//! Provides HTTP endpoints for querying balance sheet reports.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use crate::AppState;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::balance_sheet_service::{self, BalanceSheetResponse};

/// Query parameters for balance sheet endpoint
#[derive(Debug, Deserialize)]
pub struct BalanceSheetQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Accounting period ID
    pub period_id: Uuid,
    /// Currency code (ISO 4217, required) - e.g., "USD", "EUR"
    pub currency: String,
}

/// Balance sheet error response
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/gl/balance-sheet
///
/// Returns balance sheet for a tenant and period with required currency.
pub async fn get_balance_sheet(
    State(app_state): State<Arc<AppState>>,
    Query(params): Query<BalanceSheetQuery>,
) -> Result<Json<BalanceSheetResponse>, BalanceSheetErrorResponse> {
    // Query balance sheet (service layer handles all transformation and totals calculation)
    let response = balance_sheet_service::get_balance_sheet(
        &app_state.pool,
        &params.tenant_id,
        params.period_id,
        &params.currency,
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            balance_sheet_service::BalanceSheetError::InvalidTenantId(_) => {
                StatusCode::BAD_REQUEST
            }
            balance_sheet_service::BalanceSheetError::Unbalanced { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            balance_sheet_service::BalanceSheetError::StatementRepo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        BalanceSheetErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct BalanceSheetErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for BalanceSheetErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

//! Income Statement API Routes (Phase 14.7)
//!
//! Provides HTTP endpoints for querying income statement (P&L) reports.

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

use crate::services::income_statement_service::{self, IncomeStatementResponse};

/// Query parameters for income statement endpoint
#[derive(Debug, Deserialize)]
pub struct IncomeStatementQuery {
    /// Tenant identifier
    pub tenant_id: String,
    /// Accounting period ID
    pub period_id: Uuid,
    /// Currency code (ISO 4217, required) - e.g., "USD", "EUR"
    pub currency: String,
}

/// Income statement error response
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Handler for GET /api/gl/income-statement
///
/// Returns income statement (P&L) for a tenant and period with required currency.
pub async fn get_income_statement(
    State(pool): State<Arc<PgPool>>,
    Query(params): Query<IncomeStatementQuery>,
) -> Result<Json<IncomeStatementResponse>, IncomeStatementErrorResponse> {
    // Query income statement (service layer handles all transformation and totals calculation)
    let response = income_statement_service::get_income_statement(
        &pool,
        &params.tenant_id,
        params.period_id,
        &params.currency,
    )
    .await
    .map_err(|e| {
        // Map service errors to appropriate HTTP status codes
        let status = match e {
            income_statement_service::IncomeStatementError::InvalidTenantId(_) => {
                StatusCode::BAD_REQUEST
            }
            income_statement_service::IncomeStatementError::AccountingEquationViolation { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            income_statement_service::IncomeStatementError::StatementRepo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        IncomeStatementErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

/// Error response wrapper for proper HTTP error handling
#[derive(Debug)]
pub struct IncomeStatementErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for IncomeStatementErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

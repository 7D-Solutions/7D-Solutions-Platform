//! Reporting Currency Statement Routes (Phase 23a, bd-2fu)
//!
//! Provides HTTP endpoints for querying financial statements in the tenant's
//! reporting (functional) currency. These endpoints return stored reporting
//! currency amounts — including realized FX gain/loss from settlement and
//! unrealized FX revaluation from period close — NOT live conversions.
//!
//! Stability guarantee: amounts are deterministic and reconcile to journal
//! postings and period close snapshots.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;
use crate::services::balance_sheet_service::{self, BalanceSheetResponse};
use crate::services::income_statement_service::{self, IncomeStatementResponse};
use crate::services::trial_balance_service::{self, TrialBalanceResponse};

/// Query parameters for reporting currency statement endpoints.
///
/// `reporting_currency` is the tenant's functional currency (e.g. "USD").
/// The response contains only stored amounts in this currency — no live FX conversion.
#[derive(Debug, Deserialize)]
pub struct ReportingCurrencyQuery {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub reporting_currency: String,
}

/// Wrapper response that adds reporting currency metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingTrialBalanceResponse {
    pub reporting_currency: String,
    pub is_reporting_currency: bool,
    #[serde(flatten)]
    pub statement: TrialBalanceResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingIncomeStatementResponse {
    pub reporting_currency: String,
    pub is_reporting_currency: bool,
    #[serde(flatten)]
    pub statement: IncomeStatementResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingBalanceSheetResponse {
    pub reporting_currency: String,
    pub is_reporting_currency: bool,
    #[serde(flatten)]
    pub statement: BalanceSheetResponse,
}

/// Error response for reporting currency endpoints.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug)]
pub struct ReportingErrorResponse {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for ReportingErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

/// Validate reporting currency format (ISO 4217: 3 uppercase letters).
fn validate_reporting_currency(currency: &str) -> Result<(), ReportingErrorResponse> {
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(ReportingErrorResponse {
            status: StatusCode::BAD_REQUEST,
            message: format!(
                "Invalid reporting_currency '{}': must be 3 uppercase letters (ISO 4217)",
                currency
            ),
        });
    }
    Ok(())
}

/// GET /api/gl/reporting/trial-balance
///
/// Returns the trial balance in the tenant's reporting currency.
/// Amounts are from stored postings (original reporting-currency entries
/// + realized FX gain/loss + unrealized FX revaluation). No live conversion.
pub async fn get_reporting_trial_balance(
    State(app_state): State<Arc<AppState>>,
    Query(params): Query<ReportingCurrencyQuery>,
) -> Result<Json<ReportingTrialBalanceResponse>, ReportingErrorResponse> {
    validate_reporting_currency(&params.reporting_currency)?;

    let statement = trial_balance_service::get_trial_balance(
        &app_state.pool,
        &params.tenant_id,
        params.period_id,
        &params.reporting_currency,
    )
    .await
    .map_err(|e| {
        let status = match &e {
            trial_balance_service::TrialBalanceError::InvalidTenantId(_) => StatusCode::BAD_REQUEST,
            trial_balance_service::TrialBalanceError::Unbalanced { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            trial_balance_service::TrialBalanceError::StatementRepo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        ReportingErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(ReportingTrialBalanceResponse {
        reporting_currency: params.reporting_currency.clone(),
        is_reporting_currency: true,
        statement,
    }))
}

/// GET /api/gl/reporting/income-statement
///
/// Returns the income statement (P&L) in the tenant's reporting currency.
/// Includes FX gain/loss accounts from revaluation and realized settlement.
pub async fn get_reporting_income_statement(
    State(app_state): State<Arc<AppState>>,
    Query(params): Query<ReportingCurrencyQuery>,
) -> Result<Json<ReportingIncomeStatementResponse>, ReportingErrorResponse> {
    validate_reporting_currency(&params.reporting_currency)?;

    let statement = income_statement_service::get_income_statement(
        &app_state.pool,
        &params.tenant_id,
        params.period_id,
        &params.reporting_currency,
    )
    .await
    .map_err(|e| {
        let status = match &e {
            income_statement_service::IncomeStatementError::InvalidTenantId(_) => {
                StatusCode::BAD_REQUEST
            }
            income_statement_service::IncomeStatementError::AccountingEquationViolation {
                ..
            } => StatusCode::INTERNAL_SERVER_ERROR,
            income_statement_service::IncomeStatementError::StatementRepo(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        ReportingErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(ReportingIncomeStatementResponse {
        reporting_currency: params.reporting_currency.clone(),
        is_reporting_currency: true,
        statement,
    }))
}

/// GET /api/gl/reporting/balance-sheet
///
/// Returns the balance sheet in the tenant's reporting currency.
/// Reflects all FX adjustments: realized gain/loss from settlements
/// and unrealized revaluation from period close.
pub async fn get_reporting_balance_sheet(
    State(app_state): State<Arc<AppState>>,
    Query(params): Query<ReportingCurrencyQuery>,
) -> Result<Json<ReportingBalanceSheetResponse>, ReportingErrorResponse> {
    validate_reporting_currency(&params.reporting_currency)?;

    let statement = balance_sheet_service::get_balance_sheet(
        &app_state.pool,
        &params.tenant_id,
        params.period_id,
        &params.reporting_currency,
    )
    .await
    .map_err(|e| {
        let status = match &e {
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
        ReportingErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(ReportingBalanceSheetResponse {
        reporting_currency: params.reporting_currency.clone(),
        is_reporting_currency: true,
        statement,
    }))
}

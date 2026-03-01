//! Cash Flow Statement Route (Phase 24b, bd-2w3)
//!
//! GET /api/gl/cash-flow
//!
//! Returns cash flow statement derived from GL journal lines, classified
//! into operating / investing / financing via account tagging.
//! Includes reconciliation check against cash account balance deltas.

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
use crate::services::cashflow_service::{self, CashFlowResponse};
use crate::AppState;

/// Query parameters for the cash flow endpoint.
#[derive(Debug, Deserialize)]
pub struct CashFlowQuery {
    pub period_id: Uuid,
    pub currency: String,
    /// Comma-separated list of account codes designated as cash accounts
    /// for reconciliation (e.g., "1000,1010").
    #[serde(default)]
    pub cash_accounts: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ErrorResponse {
    error: String,
}

pub struct CashFlowErrorResponse {
    status: StatusCode,
    message: String,
}

impl IntoResponse for CashFlowErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (self.status, body).into_response()
    }
}

/// GET /api/gl/cash-flow
///
/// Returns cash flow statement with operating/investing/financing breakdown.
/// Cash flow is derived from GL journal entries for classified accounts.
/// Includes reconciliation: net cash flow should equal cash account net change.
pub async fn get_cash_flow(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<CashFlowQuery>,
) -> Result<Json<CashFlowResponse>, CashFlowErrorResponse> {
    let tenant_id = extract_tenant(&claims).map_err(|(_, msg)| CashFlowErrorResponse {
        status: StatusCode::UNAUTHORIZED,
        message: msg,
    })?;

    // Validate currency format
    if params.currency.len() != 3 || !params.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(CashFlowErrorResponse {
            status: StatusCode::BAD_REQUEST,
            message: format!(
                "Invalid currency '{}': must be 3 uppercase letters (ISO 4217)",
                params.currency
            ),
        });
    }

    // Parse cash account codes from comma-separated string
    let cash_account_codes: Vec<String> = params
        .cash_accounts
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let response = cashflow_service::get_cash_flow(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        &params.currency,
        &cash_account_codes,
    )
    .await
    .map_err(|e| {
        let status = match &e {
            cashflow_service::CashFlowError::InvalidTenantId(_) => StatusCode::BAD_REQUEST,
            cashflow_service::CashFlowError::InvalidCurrency(_) => StatusCode::BAD_REQUEST,
            cashflow_service::CashFlowError::PeriodNotFound { .. } => StatusCode::NOT_FOUND,
            cashflow_service::CashFlowError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        CashFlowErrorResponse {
            status,
            message: e.to_string(),
        }
    })?;

    Ok(Json(response))
}

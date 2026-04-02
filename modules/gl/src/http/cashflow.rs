//! Cash Flow Statement Route (Phase 24b, bd-2w3)
//!
//! GET /api/gl/cash-flow
//!
//! Returns cash flow statement derived from GL journal lines, classified
//! into operating / investing / financing via account tagging.
//! Includes reconciliation check against cash account balance deltas.

use axum::{extract::{Query, State}, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::auth::with_request_id;
use crate::services::cashflow_service::{self, CashFlowResponse};
use crate::AppState;

/// Query parameters for the cash flow endpoint.
#[derive(Debug, Deserialize)]
pub struct CashFlowQuery {
    pub period_id: Uuid,
    pub currency: String,
    #[serde(default)]
    pub cash_accounts: Option<String>,
}

#[utoipa::path(get, path = "/api/gl/cash-flow", tag = "Financial Statements",
    responses((status = 200, description = "Cash flow statement", body = CashFlowResponse)),
    security(("bearer" = [])))]
pub async fn get_cash_flow(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<CashFlowQuery>,
) -> Result<Json<CashFlowResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    if params.currency.len() != 3 || !params.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(with_request_id(
            ApiError::bad_request(format!(
                "Invalid currency '{}': must be 3 uppercase letters (ISO 4217)",
                params.currency
            )),
            &ctx,
        ));
    }

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
        let api_err = match &e {
            cashflow_service::CashFlowError::InvalidTenantId(_)
            | cashflow_service::CashFlowError::InvalidCurrency(_) => {
                ApiError::bad_request(e.to_string())
            }
            cashflow_service::CashFlowError::PeriodNotFound { .. } => {
                ApiError::not_found(e.to_string())
            }
            cashflow_service::CashFlowError::Database(_) => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}

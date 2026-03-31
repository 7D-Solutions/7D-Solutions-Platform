//! Reporting Currency Statement Routes (Phase 23a, bd-2fu)
//!
//! Provides HTTP endpoints for querying financial statements in the tenant's
//! reporting (functional) currency.

use axum::{extract::{Query, State}, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::auth::{extract_tenant, with_request_id};
use crate::services::balance_sheet_service::{self, BalanceSheetResponse};
use crate::services::income_statement_service::{self, IncomeStatementResponse};
use crate::services::trial_balance_service::{self, TrialBalanceResponse};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ReportingCurrencyQuery {
    pub period_id: Uuid,
    pub reporting_currency: String,
}

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

fn validate_reporting_currency(currency: &str, ctx: &Option<Extension<TracingContext>>) -> Result<(), ApiError> {
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(with_request_id(
            ApiError::bad_request(format!(
                "Invalid reporting_currency '{}': must be 3 uppercase letters (ISO 4217)",
                currency
            )),
            ctx,
        ));
    }
    Ok(())
}

pub async fn get_reporting_trial_balance(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ReportingCurrencyQuery>,
) -> Result<Json<ReportingTrialBalanceResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    validate_reporting_currency(&params.reporting_currency, &ctx)?;

    let statement = trial_balance_service::get_trial_balance(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        &params.reporting_currency,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            trial_balance_service::TrialBalanceError::InvalidTenantId(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(ReportingTrialBalanceResponse {
        reporting_currency: params.reporting_currency.clone(),
        is_reporting_currency: true,
        statement,
    }))
}

pub async fn get_reporting_income_statement(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ReportingCurrencyQuery>,
) -> Result<Json<ReportingIncomeStatementResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    validate_reporting_currency(&params.reporting_currency, &ctx)?;

    let statement = income_statement_service::get_income_statement(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        &params.reporting_currency,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            income_statement_service::IncomeStatementError::InvalidTenantId(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(ReportingIncomeStatementResponse {
        reporting_currency: params.reporting_currency.clone(),
        is_reporting_currency: true,
        statement,
    }))
}

pub async fn get_reporting_balance_sheet(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ReportingCurrencyQuery>,
) -> Result<Json<ReportingBalanceSheetResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;
    validate_reporting_currency(&params.reporting_currency, &ctx)?;

    let statement = balance_sheet_service::get_balance_sheet(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        &params.reporting_currency,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            balance_sheet_service::BalanceSheetError::InvalidTenantId(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(ReportingBalanceSheetResponse {
        reporting_currency: params.reporting_currency.clone(),
        is_reporting_currency: true,
        statement,
    }))
}

//! Income Statement API Routes (Phase 14.7)
//!
//! Provides HTTP endpoints for querying income statement (P&L) reports.

use crate::AppState;
use axum::{extract::{Query, State}, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::auth::with_request_id;
use crate::services::income_statement_service::{self, IncomeStatementResponse};

#[derive(Debug, Deserialize)]
pub struct IncomeStatementQuery {
    pub period_id: Uuid,
    pub currency: String,
}

#[utoipa::path(get, path = "/api/gl/income-statement", tag = "Financial Statements",
    responses((status = 200, description = "Income statement report", body = IncomeStatementResponse)),
    security(("bearer" = [])))]
pub async fn get_income_statement(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<IncomeStatementQuery>,
) -> Result<Json<IncomeStatementResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let response = income_statement_service::get_income_statement(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        &params.currency,
    )
    .await
    .map_err(|e| {
        let api_err = match e {
            income_statement_service::IncomeStatementError::InvalidTenantId(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}

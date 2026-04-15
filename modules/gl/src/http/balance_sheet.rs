//! Balance Sheet API Routes (Phase 14.7)
//!
//! Provides HTTP endpoints for querying balance sheet reports.

use crate::AppState;
use axum::{
    extract::{Query, State},
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::auth::with_request_id;
use crate::services::balance_sheet_service::{self, BalanceSheetResponse};
use platform_sdk::extract_tenant;

/// Query parameters for balance sheet endpoint
#[derive(Debug, Deserialize)]
pub struct BalanceSheetQuery {
    pub period_id: Uuid,
    pub currency: String,
}

#[utoipa::path(get, path = "/api/gl/balance-sheet", tag = "Financial Statements",
    responses((status = 200, description = "Balance sheet report", body = BalanceSheetResponse)),
    security(("bearer" = [])))]
pub async fn get_balance_sheet(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<BalanceSheetQuery>,
) -> Result<Json<BalanceSheetResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let response = balance_sheet_service::get_balance_sheet(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        &params.currency,
    )
    .await
    .map_err(|e| {
        let api_err = match e {
            balance_sheet_service::BalanceSheetError::InvalidTenantId(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}

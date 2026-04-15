//! Trial Balance API Routes
//!
//! Provides HTTP endpoints for querying trial balance reports.

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
use crate::services::trial_balance_service::{self, TrialBalanceResponse};
use platform_sdk::extract_tenant;

/// Query parameters for trial balance endpoint
#[derive(Debug, Deserialize)]
pub struct TrialBalanceQuery {
    /// Accounting period ID
    pub period_id: Uuid,
    /// Currency code (ISO 4217, optional) - e.g., "USD", "EUR". If omitted, all currencies are returned.
    pub currency: Option<String>,
}

/// Handler for GET /api/gl/trial-balance
///
/// Returns trial balance for a tenant and period with optional currency filter.
/// Tenant identity is derived from JWT claims (VerifiedClaims).
#[utoipa::path(get, path = "/api/gl/trial-balance", tag = "Financial Statements",
    responses((status = 200, description = "Trial balance report", body = TrialBalanceResponse)),
    security(("bearer" = [])))]
pub async fn get_trial_balance(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<TrialBalanceQuery>,
) -> Result<Json<TrialBalanceResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let response = trial_balance_service::get_trial_balance(
        &app_state.pool,
        &tenant_id,
        params.period_id,
        params.currency.as_deref().unwrap_or("USD"),
    )
    .await
    .map_err(|e| {
        let api_err = match e {
            trial_balance_service::TrialBalanceError::InvalidTenantId(_) => {
                ApiError::bad_request(e.to_string())
            }
            _ => ApiError::internal(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}

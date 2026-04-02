//! Account Activity API Routes
//!
//! Provides HTTP endpoints for querying account activity (journal lines for a specific account).

use crate::AppState;
use axum::{extract::{Path, Query, State}, Extension, Json};
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::auth::{extract_tenant, with_request_id};
use crate::services::account_activity_service::{self, AccountActivityResponse};

/// Query parameters for account activity endpoint
#[derive(Debug, Deserialize)]
pub struct AccountActivityQuery {
    pub period_id: Option<Uuid>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub currency: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[utoipa::path(get, path = "/api/gl/accounts/{account_code}/activity", tag = "GL Detail",
    params(("account_code" = String, Path, description = "Account code")),
    responses((status = 200, description = "Account activity lines", body = AccountActivityResponse)),
    security(("bearer" = [])))]
pub async fn get_account_activity(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(account_code): Path<String>,
    Query(params): Query<AccountActivityQuery>,
) -> Result<Json<AccountActivityResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let response = account_activity_service::get_account_activity(
        &app_state.pool,
        &tenant_id,
        &account_code,
        params.period_id,
        params.start_date,
        params.end_date,
        params.currency.as_deref(),
        params.limit,
        params.offset,
    )
    .await
    .map_err(|e| {
        let api_err = match &e {
            account_activity_service::AccountActivityServiceError::PeriodNotFound { .. } => {
                ApiError::not_found(e.to_string())
            }
            account_activity_service::AccountActivityServiceError::Repo(_) => {
                ApiError::internal(e.to_string())
            }
            _ => ApiError::bad_request(e.to_string()),
        };
        with_request_id(api_err, &ctx)
    })?;

    Ok(Json(response))
}

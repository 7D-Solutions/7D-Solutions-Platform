//! HTTP handlers for financial statement endpoints.
//!
//! Endpoints (all GET, read-only):
//!   GET /api/reporting/pl?from=YYYY-MM-DD&to=YYYY-MM-DD
//!   GET /api/reporting/balance-sheet?as_of=YYYY-MM-DD
//!
//! Both endpoints read directly from `rpt_trial_balance_cache` via domain
//! functions. No GL writes, no on-demand recalculation from raw events.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use super::tenant::{extract_tenant, with_request_id};
use crate::domain::statements::{balance_sheet, pl};

// ── Query parameter structs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PlParams {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BsParams {
    pub as_of: NaiveDate,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/pl — Profit & Loss statement for a date range.
#[utoipa::path(
    get,
    path = "/api/reporting/pl",
    tag = "Statements",
    params(PlParams),
    responses(
        (status = 200, description = "P&L statement", body = pl::PlStatement),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["REPORTING_READ"]))
)]
pub async fn get_pl(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<PlParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match pl::compute_pl(&state.pool, &tenant_id, params.from, params.to).await {
        Ok(stmt) => Json(stmt).into_response(),
        Err(e) => {
            tracing::error!(tenant_id = %tenant_id, error = %e, "P&L computation failed");
            let api_err = ApiError::internal("P&L computation failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

/// GET /api/reporting/balance-sheet — Balance Sheet as of a given date.
#[utoipa::path(
    get,
    path = "/api/reporting/balance-sheet",
    tag = "Statements",
    params(BsParams),
    responses(
        (status = 200, description = "Balance sheet", body = balance_sheet::BalanceSheet),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["REPORTING_READ"]))
)]
pub async fn get_balance_sheet(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<BsParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match balance_sheet::compute_balance_sheet(&state.pool, &tenant_id, params.as_of).await {
        Ok(stmt) => Json(stmt).into_response(),
        Err(e) => {
            tracing::error!(tenant_id = %tenant_id, error = %e, "Balance sheet computation failed");
            let api_err = ApiError::internal("Balance sheet computation failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

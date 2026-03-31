//! HTTP handler for the cash flow statement endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/cashflow?from=YYYY-MM-DD&to=YYYY-MM-DD
//!
//! Returns operating / investing / financing sections with per-line and
//! per-currency totals, plus net cash change by currency.

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

use crate::domain::statements::cashflow;

use super::tenant::{extract_tenant, with_request_id};

// ── Query parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CashflowParams {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/cashflow — Cash flow statement for a date range.
#[utoipa::path(
    get,
    path = "/api/reporting/cashflow",
    tag = "Statements",
    params(CashflowParams),
    responses(
        (status = 200, description = "Cash flow statement", body = cashflow::CashflowStatement),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["REPORTING_READ"]))
)]
pub async fn get_cashflow(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<CashflowParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match cashflow::compute_cashflow(&state.pool, &tenant_id, params.from, params.to).await {
        Ok(stmt) => Json(stmt).into_response(),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Cash flow computation failed"
            );
            let api_err = ApiError::internal("Cash flow computation failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

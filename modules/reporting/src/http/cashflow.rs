//! HTTP handler for the cash flow statement endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/cashflow?tenant_id=...&from=YYYY-MM-DD&to=YYYY-MM-DD
//!
//! Returns operating / investing / financing sections with per-line and
//! per-currency totals, plus net cash change by currency.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::statements::cashflow;

// ── Query parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CashflowParams {
    pub tenant_id: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/cashflow — Cash flow statement for a date range.
///
/// Query params:
///   - tenant_id: tenant scope
///   - from: period start (YYYY-MM-DD, inclusive)
///   - to: period end   (YYYY-MM-DD, inclusive)
///
/// Returns operating, investing, and financing sections with per-line
/// detail, per-currency section totals, and net cash change by currency.
pub async fn get_cashflow(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<CashflowParams>,
) -> Result<Json<cashflow::CashflowStatement>, (StatusCode, String)> {
    cashflow::compute_cashflow(&state.pool, &params.tenant_id, params.from, params.to)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %params.tenant_id,
                error = %e,
                "Cash flow computation failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

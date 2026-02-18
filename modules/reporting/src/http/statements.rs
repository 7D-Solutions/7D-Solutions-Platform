//! HTTP handlers for financial statement endpoints.
//!
//! Endpoints (all GET, read-only):
//!   GET /api/reporting/pl?tenant_id=...&from=YYYY-MM-DD&to=YYYY-MM-DD
//!   GET /api/reporting/balance-sheet?tenant_id=...&as_of=YYYY-MM-DD
//!
//! Both endpoints read directly from `rpt_trial_balance_cache` via domain
//! functions. No GL writes, no on-demand recalculation from raw events.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::statements::{balance_sheet, pl};

// ── Query parameter structs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PlParams {
    pub tenant_id: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Deserialize)]
pub struct BsParams {
    pub tenant_id: String,
    pub as_of: NaiveDate,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/pl — Profit & Loss statement for a date range.
///
/// Query params:
///   - tenant_id: tenant scope
///   - from: period start (YYYY-MM-DD, inclusive)
///   - to: period end   (YYYY-MM-DD, inclusive)
///
/// Returns revenue, COGS, and expenses sections with per-account and
/// per-currency totals, plus net income by currency.
pub async fn get_pl(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<PlParams>,
) -> Result<Json<pl::PlStatement>, (StatusCode, String)> {
    pl::compute_pl(&state.pool, &params.tenant_id, params.from, params.to)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(tenant_id = %params.tenant_id, error = %e, "P&L computation failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

/// GET /api/reporting/balance-sheet — Balance Sheet as of a given date.
///
/// Query params:
///   - tenant_id: tenant scope
///   - as_of: point-in-time date (YYYY-MM-DD, inclusive — cumulative)
///
/// Returns assets, liabilities, and equity sections with per-account and
/// per-currency totals.
pub async fn get_balance_sheet(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<BsParams>,
) -> Result<Json<balance_sheet::BalanceSheet>, (StatusCode, String)> {
    balance_sheet::compute_balance_sheet(&state.pool, &params.tenant_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(tenant_id = %params.tenant_id, error = %e, "Balance sheet computation failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

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
    http::StatusCode,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use super::admin_types::ErrorBody;
use crate::domain::statements::{balance_sheet, pl};

// ── Auth helper ─────────────────────────────────────────────────────────────

pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, String)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            "Missing or invalid authentication".to_string(),
        )),
    }
}

// ── Query parameter structs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PlParams {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Deserialize)]
pub struct BsParams {
    pub as_of: NaiveDate,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/pl — Profit & Loss statement for a date range.
pub async fn get_pl(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<PlParams>,
) -> Result<Json<pl::PlStatement>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|(status, msg)| (status, Json(ErrorBody::new("unauthorized", &msg))))?;

    pl::compute_pl(&state.pool, &tenant_id, params.from, params.to)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(tenant_id = %tenant_id, error = %e, "P&L computation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", e.to_string())),
            )
        })
}

/// GET /api/reporting/balance-sheet — Balance Sheet as of a given date.
pub async fn get_balance_sheet(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<BsParams>,
) -> Result<Json<balance_sheet::BalanceSheet>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|(status, msg)| (status, Json(ErrorBody::new("unauthorized", &msg))))?;

    balance_sheet::compute_balance_sheet(&state.pool, &tenant_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(tenant_id = %tenant_id, error = %e, "Balance sheet computation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", e.to_string())),
            )
        })
}

//! HTTP handler for the cash flow statement endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/cashflow?from=YYYY-MM-DD&to=YYYY-MM-DD
//!
//! Returns operating / investing / financing sections with per-line and
//! per-currency totals, plus net cash change by currency.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::statements::cashflow;

use super::admin_types::ErrorBody;
use super::statements::extract_tenant;

// ── Query parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CashflowParams {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/cashflow — Cash flow statement for a date range.
pub async fn get_cashflow(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<CashflowParams>,
) -> Result<Json<cashflow::CashflowStatement>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|(status, msg)| (status, Json(ErrorBody::new("unauthorized", &msg))))?;

    cashflow::compute_cashflow(&state.pool, &tenant_id, params.from, params.to)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Cash flow computation failed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", e.to_string())),
            )
        })
}

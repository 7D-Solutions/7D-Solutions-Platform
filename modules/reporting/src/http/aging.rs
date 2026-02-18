//! HTTP handler for the AP aging endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/ap-aging?tenant_id=...&as_of=YYYY-MM-DD
//!
//! Returns per-vendor aging buckets and summary totals by currency,
//! read from the `rpt_ap_aging_cache`.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::aging::ap_aging;

// ── Query parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApAgingParams {
    pub tenant_id: String,
    pub as_of: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/ap-aging — AP aging report from reporting cache.
///
/// Query params:
///   - tenant_id: tenant scope
///   - as_of: snapshot date (YYYY-MM-DD)
///
/// Returns per-vendor aging buckets (current/30/60/90/over_90) and
/// summary totals by currency.
pub async fn get_ap_aging(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<ApAgingParams>,
) -> Result<Json<ap_aging::ApAgingReport>, (StatusCode, String)> {
    ap_aging::query_ap_aging(&state.pool, &params.tenant_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %params.tenant_id,
                as_of = %params.as_of,
                error = %e,
                "AP aging query failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

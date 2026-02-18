//! HTTP handlers for aging endpoints (AR and AP).
//!
//! Endpoints:
//!   GET /api/reporting/ar-aging?tenant_id=...&as_of=YYYY-MM-DD
//!   GET /api/reporting/ap-aging?tenant_id=...&as_of=YYYY-MM-DD
//!
//! Returns aging buckets (current/30/60/90/90+) from the reporting cache,
//! aggregated per currency.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::aging::{ap_aging, ar_aging};

// ── AR aging ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ArAgingParams {
    pub tenant_id: String,
    pub as_of: NaiveDate,
}

#[derive(Debug, Serialize)]
pub struct ArAgingResponse {
    pub tenant_id: String,
    pub as_of: NaiveDate,
    pub aging: Vec<ar_aging::ArAgingSummary>,
}

/// GET /api/reporting/ar-aging — AR aging buckets from the reporting cache.
///
/// Query params:
///   - tenant_id: tenant scope
///   - as_of: point-in-time date (YYYY-MM-DD)
///
/// Returns per-currency aging summaries with current, 1-30, 31-60, 61-90,
/// and 90+ day buckets, plus total outstanding.
pub async fn get_ar_aging(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<ArAgingParams>,
) -> Result<Json<ArAgingResponse>, (StatusCode, String)> {
    let aging = ar_aging::get_aging_summary(&state.pool, &params.tenant_id, params.as_of)
        .await
        .map_err(|e| {
            tracing::error!(
                tenant_id = %params.tenant_id,
                as_of = %params.as_of,
                error = %e,
                "AR aging query failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(ArAgingResponse {
        tenant_id: params.tenant_id,
        as_of: params.as_of,
        aging,
    }))
}

// ── AP aging ─────────────────────────────────────────────────────────────────

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

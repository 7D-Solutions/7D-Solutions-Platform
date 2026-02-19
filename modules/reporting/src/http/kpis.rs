//! HTTP handler for the unified KPI endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/kpis?tenant_id=...&as_of=YYYY-MM-DD
//!
//! Returns pre-computed KPI values sourced from reporting caches.
//! All values are grouped by currency. Missing KPIs return empty maps.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::kpis::{compute_kpis, KpiSnapshot};

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct KpiParams {
    pub tenant_id: String,
    pub as_of: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/kpis — unified KPI snapshot from reporting caches.
///
/// Query params:
///   - tenant_id: tenant scope
///   - as_of: snapshot date (YYYY-MM-DD)
///
/// Returns KPIs sourced from pre-computed cache tables:
///   - ar_total_outstanding: AR outstanding by currency
///   - ap_total_outstanding: AP outstanding by currency
///   - cash_collected_ytd: operating cash inflows YTD by currency
///   - burn_ytd: expense account totals YTD by currency
///   - mrr: monthly recurring revenue (if ingested)
///   - inventory_value: inventory valuation (if ingested)
pub async fn get_kpis(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<KpiParams>,
) -> Result<Json<KpiSnapshot>, (StatusCode, String)> {
    compute_kpis(&state.pool, &params.tenant_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %params.tenant_id,
                as_of = %params.as_of,
                error = %e,
                "KPI computation failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

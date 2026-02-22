//! HTTP handler for the probabilistic cash forecast endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/forecast?tenant_id=...&horizons=7,14,30,60,90
//!
//! Returns currency-grouped expected collections with confidence bands
//! and an at-risk invoice list.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::forecast::{compute_cash_forecast, CashForecastResponse};

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ForecastParams {
    pub tenant_id: String,
    /// Comma-separated horizon days (e.g. "7,14,30,60,90").
    /// Defaults to "7,14,30,60,90" if omitted.
    pub horizons: Option<String>,
}

impl ForecastParams {
    fn parse_horizons(&self) -> Vec<u32> {
        match &self.horizons {
            Some(s) => s
                .split(',')
                .filter_map(|v| v.trim().parse::<u32>().ok())
                .collect(),
            None => vec![7, 14, 30, 60, 90],
        }
    }
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/forecast — probabilistic cash collection forecast.
pub async fn get_forecast(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<ForecastParams>,
) -> Result<Json<CashForecastResponse>, (StatusCode, String)> {
    let horizons = params.parse_horizons();
    if horizons.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No valid horizons provided".into()));
    }

    compute_cash_forecast(&state.pool, &params.tenant_id, &horizons)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %params.tenant_id,
                error = %e,
                "Forecast computation failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

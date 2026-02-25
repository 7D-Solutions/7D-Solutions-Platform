//! HTTP handler for the probabilistic cash forecast endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/forecast?horizons=7,14,30,60,90
//!
//! Returns currency-grouped expected collections with confidence bands
//! and an at-risk invoice list.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::forecast::{compute_cash_forecast, CashForecastResponse};

use super::statements::extract_tenant;
use super::admin_types::ErrorBody;

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ForecastParams {
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
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ForecastParams>,
) -> Result<Json<CashForecastResponse>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims).map_err(|(status, msg)| {
        (status, Json(ErrorBody::new("unauthorized", &msg)))
    })?;
    
    let horizons = params.parse_horizons();
    if horizons.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorBody::new("validation_error", "No valid horizons provided"))));
    }

    compute_cash_forecast(&state.pool, &tenant_id, &horizons)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Forecast computation failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::new("internal_error", e.to_string())))
        })
}

//! HTTP handler for the probabilistic cash forecast endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/forecast?horizons=7,14,30,60,90
//!
//! Returns currency-grouped expected collections with confidence bands
//! and an at-risk invoice list.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::forecast::{compute_cash_forecast, CashForecastResponse};

use super::tenant::{extract_tenant, with_request_id};

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
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
#[utoipa::path(
    get,
    path = "/api/reporting/forecast",
    tag = "Forecast",
    params(ForecastParams),
    responses(
        (status = 200, description = "Cash forecast", body = CashForecastResponse),
        (status = 400, description = "Bad request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["REPORTING_READ"]))
)]
pub async fn get_forecast(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ForecastParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let horizons = params.parse_horizons();
    if horizons.is_empty() {
        let api_err = ApiError::bad_request("No valid horizons provided");
        return with_request_id(api_err, &tracing_ctx).into_response();
    }

    match compute_cash_forecast(&state.pool, &tenant_id, &horizons).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Forecast computation failed"
            );
            let api_err = ApiError::internal("Forecast computation failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

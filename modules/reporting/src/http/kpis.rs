//! HTTP handler for the unified KPI endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/kpis?as_of=YYYY-MM-DD
//!
//! Returns pre-computed KPI values sourced from reporting caches.
//! All values are grouped by currency. Missing KPIs return empty maps.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::kpis::{compute_kpis, KpiSnapshot};

use super::tenant::with_request_id;
use platform_sdk::extract_tenant;

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KpiParams {
    pub as_of: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/kpis — unified KPI snapshot from reporting caches.
#[utoipa::path(
    get,
    path = "/api/reporting/kpis",
    tag = "KPIs",
    params(KpiParams),
    responses(
        (status = 200, description = "KPI snapshot", body = KpiSnapshot),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["REPORTING_READ"]))
)]
pub async fn get_kpis(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<KpiParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match compute_kpis(&state.pool, &tenant_id, params.as_of).await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                as_of = %params.as_of,
                error = %e,
                "KPI computation failed"
            );
            let api_err = ApiError::internal("KPI computation failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

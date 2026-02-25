//! HTTP handler for the unified KPI endpoint.
//!
//! Endpoint:
//!   GET /api/reporting/kpis?as_of=YYYY-MM-DD
//!
//! Returns pre-computed KPI values sourced from reporting caches.
//! All values are grouped by currency. Missing KPIs return empty maps.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::kpis::{compute_kpis, KpiSnapshot};

use super::statements::extract_tenant;
use super::admin_types::ErrorBody;

// ── Query params ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct KpiParams {
    pub as_of: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/kpis — unified KPI snapshot from reporting caches.
pub async fn get_kpis(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<KpiParams>,
) -> Result<Json<KpiSnapshot>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims).map_err(|(status, msg)| {
        (status, Json(ErrorBody::new("unauthorized", &msg)))
    })?;
    
    compute_kpis(&state.pool, &tenant_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %tenant_id,
                as_of = %params.as_of,
                error = %e,
                "KPI computation failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::new("internal_error", e.to_string())))
        })
}

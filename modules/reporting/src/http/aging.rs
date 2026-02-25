//! HTTP handlers for aging endpoints (AR and AP).
//!
//! Endpoints:
//!   GET /api/reporting/ar-aging?as_of=YYYY-MM-DD
//!   GET /api/reporting/ap-aging?as_of=YYYY-MM-DD
//!
//! Returns aging buckets (current/30/60/90/90+) from the reporting cache,
//! aggregated per currency.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::aging::{ap_aging, ar_aging};

use super::statements::extract_tenant;

// ── AR aging ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ArAgingParams {
    pub as_of: NaiveDate,
}

#[derive(Debug, Serialize)]
pub struct ArAgingResponse {
    pub tenant_id: String,
    pub as_of: NaiveDate,
    pub aging: Vec<ar_aging::ArAgingSummary>,
}

/// GET /api/reporting/ar-aging — AR aging buckets from the reporting cache.
pub async fn get_ar_aging(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ArAgingParams>,
) -> Result<Json<ArAgingResponse>, (StatusCode, String)> {
    let tenant_id = extract_tenant(&claims)?;
    let aging = ar_aging::get_aging_summary(&state.pool, &tenant_id, params.as_of)
        .await
        .map_err(|e| {
            tracing::error!(
                tenant_id = %tenant_id,
                as_of = %params.as_of,
                error = %e,
                "AR aging query failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(ArAgingResponse {
        tenant_id,
        as_of: params.as_of,
        aging,
    }))
}

// ── AP aging ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApAgingParams {
    pub as_of: NaiveDate,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/reporting/ap-aging — AP aging report from reporting cache.
pub async fn get_ap_aging(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ApAgingParams>,
) -> Result<Json<ap_aging::ApAgingReport>, (StatusCode, String)> {
    let tenant_id = extract_tenant(&claims)?;
    ap_aging::query_ap_aging(&state.pool, &tenant_id, params.as_of)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(
                tenant_id = %tenant_id,
                as_of = %params.as_of,
                error = %e,
                "AP aging query failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })
}

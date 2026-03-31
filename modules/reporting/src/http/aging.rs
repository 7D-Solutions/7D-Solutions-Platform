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
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::aging::{ap_aging, ar_aging};

use super::tenant::{extract_tenant, with_request_id};

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
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ArAgingParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ar_aging::get_aging_summary(&state.pool, &tenant_id, params.as_of).await {
        Ok(aging) => Json(ArAgingResponse {
            tenant_id,
            as_of: params.as_of,
            aging,
        })
        .into_response(),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                as_of = %params.as_of,
                error = %e,
                "AR aging query failed"
            );
            let api_err = ApiError::internal("AR aging query failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
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
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ApAgingParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match ap_aging::query_ap_aging(&state.pool, &tenant_id, params.as_of).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                as_of = %params.as_of,
                error = %e,
                "AP aging query failed"
            );
            let api_err = ApiError::internal("AP aging query failed");
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

//! HTTP handler for AP aging report.
//!
//! GET /api/ap/aging
//!
//! Query parameters:
//!   - `as_of`     (YYYY-MM-DD, optional) — aging reference date, defaults to today
//!   - `by_vendor` (bool, optional)       — include per-vendor breakdown, defaults to false
//!
//! Tenant is identified via JWT claims (VerifiedClaims).

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension, Json,
};
use chrono::{NaiveDate, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;

use crate::domain::reports::aging::compute_aging;
use crate::http::tenant::{extract_tenant, with_request_id};
use crate::AppState;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AgingQuery {
    /// Aging reference date (YYYY-MM-DD). Defaults to today (UTC).
    pub as_of: Option<NaiveDate>,
    /// When `true`, include per-vendor breakdown in the response.
    #[serde(default)]
    pub by_vendor: bool,
}

// ============================================================================
// Handler
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/ap/aging",
    tag = "Reports",
    params(AgingQuery),
    responses(
        (status = 200, description = "AP aging report"),
    ),
    security(("bearer" = [])),
)]
/// GET /api/ap/aging
///
/// Returns AP aging bucket totals grouped by currency as of `as_of`.
/// Optionally includes a per-vendor breakdown when `by_vendor=true`.
///
/// Only bills with status `approved` or `partially_paid` and a positive
/// remaining open balance are included. Paid and voided bills are excluded.
pub async fn aging_report(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(params): Query<AgingQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let as_of = params.as_of.unwrap_or_else(|| Utc::now().date_naive());

    match compute_aging(&state.pool, &tenant_id, as_of, params.by_vendor).await {
        Ok(report) => Json(serde_json::json!({
            "as_of": report.as_of.to_string(),
            "buckets_by_currency": report.buckets_by_currency,
            "vendor_breakdown": report.vendor_breakdown,
        }))
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

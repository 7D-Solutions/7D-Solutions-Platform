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
    http::StatusCode,
    Extension, Json,
};
use chrono::{NaiveDate, Utc};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::reports::aging::{compute_aging, AgingError};
use crate::http::tenant::extract_tenant;
use crate::http::vendors::ErrorBody;
use crate::AppState;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
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
    Query(params): Query<AgingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let as_of = params
        .as_of
        .unwrap_or_else(|| Utc::now().date_naive());

    let report = compute_aging(&state.pool, &tenant_id, as_of, params.by_vendor)
        .await
        .map_err(aging_error_response)?;

    Ok(Json(serde_json::json!({
        "as_of": report.as_of.to_string(),
        "buckets_by_currency": report.buckets_by_currency,
        "vendor_breakdown": report.vendor_breakdown,
    })))
}

// ============================================================================
// Shared helpers
// ============================================================================

fn aging_error_response(e: AgingError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        AgingError::Database(e) => {
            tracing::error!(error = %e, "Database error in aging report handler");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "An internal error occurred")),
            )
        }
    }
}

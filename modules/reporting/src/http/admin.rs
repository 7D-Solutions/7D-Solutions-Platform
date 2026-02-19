//! Admin HTTP endpoints for the reporting module.
//!
//! Endpoint:
//!   POST /api/reporting/rebuild
//!
//! Triggers a snapshot rebuild for a tenant over a date range. Persists
//! P&L and Balance Sheet statement rows into `rpt_statement_cache`.
//!
//! ## Authorization
//!
//! Requires an `X-Admin-Token` header matching the `ADMIN_TOKEN` environment
//! variable. If `ADMIN_TOKEN` is not set, all rebuild requests are rejected.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::jobs::snapshot_runner::{run_snapshot, SnapshotRunResult};

// ── Request body ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RebuildRequest {
    pub tenant_id: String,
    /// Start of rebuild range (inclusive), YYYY-MM-DD.
    pub from: NaiveDate,
    /// End of rebuild range (inclusive), YYYY-MM-DD.
    pub to: NaiveDate,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// POST /api/reporting/rebuild — trigger a statement cache rebuild.
///
/// Body (JSON):
/// ```json
/// { "tenant_id": "acme", "from": "2026-01-01", "to": "2026-01-31" }
/// ```
///
/// Returns the rebuild summary on success (200 OK).
/// Returns 403 if the admin token is missing or wrong.
/// Returns 400 if the date range is invalid.
/// Returns 500 on internal errors.
pub async fn rebuild(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Json(req): Json<RebuildRequest>,
) -> Result<Json<SnapshotRunResult>, (StatusCode, String)> {
    // Admin-gate: reject if ADMIN_TOKEN is not configured or header doesn't match
    let expected = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    if expected.is_empty() {
        return Err((
            StatusCode::FORBIDDEN,
            "ADMIN_TOKEN is not configured; rebuild is disabled".to_string(),
        ));
    }
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != expected {
        return Err((StatusCode::FORBIDDEN, "Invalid admin token".to_string()));
    }

    // Validate range
    if req.from > req.to {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'from' ({}) must be <= 'to' ({})", req.from, req.to),
        ));
    }

    let result = run_snapshot(&state.pool, &req.tenant_id, req.from, req.to)
        .await
        .map_err(|e| {
            tracing::error!(
                tenant_id = %req.tenant_id,
                from = %req.from,
                to = %req.to,
                error = %e,
                "Rebuild failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    tracing::info!(
        tenant_id = %req.tenant_id,
        from = %req.from,
        to = %req.to,
        rows_upserted = result.rows_upserted,
        "Rebuild completed"
    );

    Ok(Json(result))
}

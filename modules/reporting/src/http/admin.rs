//! Admin HTTP endpoints for the reporting module.
//!
//! Endpoints:
//!   POST /api/reporting/rebuild               — snapshot cache rebuild (module-specific)
//!   POST /api/reporting/admin/projection-status — standardized cursor status
//!   POST /api/reporting/admin/consistency-check — standardized digest check
//!   GET  /api/reporting/admin/projections       — standardized projection listing
//!
//! ## Authorization
//!
//! Requires an `X-Admin-Token` header matching the `ADMIN_TOKEN` environment
//! variable. If `ADMIN_TOKEN` is not set, all admin requests are rejected.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::NaiveDate;
use projections::admin as proj_admin;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use crate::domain::jobs::snapshot_runner::{run_snapshot, SnapshotRunResult};
use super::admin_types::ErrorBody;

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
) -> Result<Json<SnapshotRunResult>, (StatusCode, Json<ErrorBody>)> {
    // Admin-gate: reject if ADMIN_TOKEN is not configured or header doesn't match
    let expected = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    if expected.is_empty() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorBody::new("forbidden", "ADMIN_TOKEN is not configured; rebuild is disabled")),
        ));
    }
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != expected {
        return Err((StatusCode::FORBIDDEN, Json(ErrorBody::new("forbidden", "Invalid admin token"))));
    }

    // Validate inputs
    if req.tenant_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", "tenant_id must not be empty")),
        ));
    }
    if req.from > req.to {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", format!("'from' ({}) must be <= 'to' ({})", req.from, req.to))),
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
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::new("internal_error", e.to_string())))
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

// ── Standardized admin endpoints ─────────────────────────────────────────────

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
}

fn guard(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ErrorBody>)> {
    proj_admin::verify_admin_token(extract_token(headers)).map_err(|msg| {
        tracing::warn!(reason = msg, "Admin request rejected");
        (StatusCode::FORBIDDEN, Json(ErrorBody::new("forbidden", msg)))
    })
}

async fn projection_status(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<proj_admin::ProjectionStatusRequest>,
) -> Result<Json<proj_admin::ProjectionStatusResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = proj_admin::query_projection_status(&pool, &req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::new("internal_error", e))))?;
    Ok(Json(resp))
}

async fn consistency_check(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<proj_admin::ConsistencyCheckRequest>,
) -> Result<Json<proj_admin::ConsistencyCheckResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = proj_admin::query_consistency_check(&pool, &req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::new("internal_error", e))))?;
    Ok(Json(resp))
}

async fn list_projections(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<proj_admin::ProjectionListResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = proj_admin::query_projection_list(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::new("internal_error", e))))?;
    Ok(Json(resp))
}

/// Build the standardized admin sub-router (state = PgPool).
pub fn admin_router(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/api/reporting/admin/projection-status",
            post(projection_status),
        )
        .route(
            "/api/reporting/admin/consistency-check",
            post(consistency_check),
        )
        .route(
            "/api/reporting/admin/projections",
            get(list_projections),
        )
        .with_state(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_admin_router_builds() {
        let pool = PgPool::connect_lazy("postgres://localhost/fake").unwrap();
        let _router = admin_router(pool);
    }
}

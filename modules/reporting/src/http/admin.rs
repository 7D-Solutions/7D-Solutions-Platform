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
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use platform_http_contracts::{ApiError, PaginatedResponse};
use projections::admin as proj_admin;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::domain::jobs::snapshot_runner::{run_snapshot, SnapshotRunResult};

// ── Request body ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct RebuildRequest {
    pub tenant_id: String,
    /// Start of rebuild range (inclusive), YYYY-MM-DD.
    pub from: NaiveDate,
    /// End of rebuild range (inclusive), YYYY-MM-DD.
    pub to: NaiveDate,
}

// ── Local ToSchema mirrors of projections::admin types ──────────────────────
// The projections crate doesn't depend on utoipa, so we define OpenAPI-visible
// versions here and convert from the upstream types.

/// Cursor status for a single projection/tenant pair.
#[derive(Debug, Serialize, ToSchema)]
pub struct CursorStatusSchema {
    pub projection_name: String,
    pub tenant_id: String,
    pub events_processed: i64,
    pub last_event_id: String,
    pub last_event_occurred_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Projection status response (may contain multiple tenant cursors).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectionStatusSchema {
    pub projection_name: String,
    pub cursors: Vec<CursorStatusSchema>,
    pub status: String,
}

/// Consistency check result.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsistencyCheckSchema {
    pub projection_name: String,
    pub table_exists: bool,
    pub row_count: i64,
    pub digest: String,
    pub digest_version: String,
    pub order_by: String,
    pub checked_at: DateTime<Utc>,
    pub status: String,
}

/// Summary of a single projection in the listing.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionSummarySchema {
    pub projection_name: String,
    pub tenant_count: i64,
    pub total_events_processed: i64,
    pub last_updated: Option<DateTime<Utc>>,
}

impl From<proj_admin::ProjectionStatusResponse> for ProjectionStatusSchema {
    fn from(r: proj_admin::ProjectionStatusResponse) -> Self {
        Self {
            projection_name: r.projection_name,
            cursors: r
                .cursors
                .into_iter()
                .map(|c| CursorStatusSchema {
                    projection_name: c.projection_name,
                    tenant_id: c.tenant_id,
                    events_processed: c.events_processed,
                    last_event_id: c.last_event_id,
                    last_event_occurred_at: c.last_event_occurred_at,
                    updated_at: c.updated_at,
                })
                .collect(),
            status: r.status.to_string(),
        }
    }
}

impl From<proj_admin::ConsistencyCheckResponse> for ConsistencyCheckSchema {
    fn from(r: proj_admin::ConsistencyCheckResponse) -> Self {
        Self {
            projection_name: r.projection_name,
            table_exists: r.table_exists,
            row_count: r.row_count,
            digest: r.digest,
            digest_version: r.digest_version,
            order_by: r.order_by,
            checked_at: r.checked_at,
            status: r.status.to_string(),
        }
    }
}

impl From<proj_admin::ProjectionSummary> for ProjectionSummarySchema {
    fn from(s: proj_admin::ProjectionSummary) -> Self {
        Self {
            projection_name: s.projection_name,
            tenant_count: s.tenant_count,
            total_events_processed: s.total_events_processed,
            last_updated: s.last_updated,
        }
    }
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// POST /api/reporting/rebuild — trigger a statement cache rebuild.
#[utoipa::path(
    post,
    path = "/api/reporting/rebuild",
    tag = "Admin",
    request_body = RebuildRequest,
    responses(
        (status = 200, description = "Rebuild completed", body = SnapshotRunResult),
        (status = 400, description = "Bad request", body = ApiError),
        (status = 403, description = "Forbidden", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["REPORTING_MUTATE"]))
)]
pub async fn rebuild(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Json(req): Json<RebuildRequest>,
) -> impl IntoResponse {
    // Admin-gate: reject if ADMIN_TOKEN is not configured or header doesn't match
    let expected = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    if expected.is_empty() {
        return ApiError::new(
            403,
            "forbidden",
            "ADMIN_TOKEN is not configured; rebuild is disabled",
        )
        .into_response();
    }
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != expected {
        return ApiError::new(403, "forbidden", "Invalid admin token").into_response();
    }

    // Validate inputs
    if req.tenant_id.trim().is_empty() {
        return ApiError::new(400, "validation_error", "tenant_id must not be empty")
            .into_response();
    }
    if req.from > req.to {
        return ApiError::new(
            400,
            "validation_error",
            format!("'from' ({}) must be <= 'to' ({})", req.from, req.to),
        )
        .into_response();
    }

    match run_snapshot(&state.pool, &req.tenant_id, req.from, req.to).await {
        Ok(result) => {
            tracing::info!(
                tenant_id = %req.tenant_id,
                from = %req.from,
                to = %req.to,
                rows_upserted = result.rows_upserted,
                "Rebuild completed"
            );
            Json(result).into_response()
        }
        Err(e) => {
            tracing::error!(
                tenant_id = %req.tenant_id,
                from = %req.from,
                to = %req.to,
                error = %e,
                "Rebuild failed"
            );
            ApiError::internal("Rebuild failed").into_response()
        }
    }
}

// ── Standardized admin endpoints ─────────────────────────────────────────────

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-admin-token").and_then(|v| v.to_str().ok())
}

fn guard(headers: &HeaderMap) -> Result<(), ApiError> {
    proj_admin::verify_admin_token(extract_token(headers)).map_err(|msg| {
        tracing::warn!(reason = msg, "Admin request rejected");
        ApiError::forbidden(msg)
    })
}

#[utoipa::path(
    post,
    path = "/api/reporting/admin/projection-status",
    tag = "Admin",
    responses(
        (status = 200, description = "Projection cursor status", body = ProjectionStatusSchema),
        (status = 403, description = "Forbidden — invalid or missing admin token", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
)]
pub async fn projection_status(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<proj_admin::ProjectionStatusRequest>,
) -> Result<Json<ProjectionStatusSchema>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = proj_admin::query_projection_status(&pool, &req)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Admin projection-status error");
            ApiError::internal("Internal error")
        })?;
    Ok(Json(resp.into()))
}

#[utoipa::path(
    post,
    path = "/api/reporting/admin/consistency-check",
    tag = "Admin",
    responses(
        (status = 200, description = "Consistency check result", body = ConsistencyCheckSchema),
        (status = 403, description = "Forbidden — invalid or missing admin token", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
)]
pub async fn consistency_check(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<proj_admin::ConsistencyCheckRequest>,
) -> Result<Json<ConsistencyCheckSchema>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = proj_admin::query_consistency_check(&pool, &req)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Admin consistency-check error");
            ApiError::internal("Internal error")
        })?;
    Ok(Json(resp.into()))
}

#[utoipa::path(
    get,
    path = "/api/reporting/admin/projections",
    tag = "Admin",
    responses(
        (status = 200, description = "Paginated list of all known projections", body = PaginatedResponse<ProjectionSummarySchema>),
        (status = 403, description = "Forbidden — invalid or missing admin token", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
)]
pub async fn list_projections(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<PaginatedResponse<ProjectionSummarySchema>>, ApiError> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = proj_admin::query_projection_list(&pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Admin list-projections error");
            ApiError::internal("Internal error")
        })?;
    let items: Vec<ProjectionSummarySchema> =
        resp.projections.into_iter().map(Into::into).collect();
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse::new(items, 1, total, total)))
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
        .route("/api/reporting/admin/projections", get(list_projections))
        .with_state(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_admin_router_builds() {
        let pool =
            PgPool::connect_lazy("postgres://localhost/fake").expect("connect_lazy for test");
        let _router = admin_router(pool);
    }
}

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
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::NaiveDate;
use platform_http_contracts::ApiError;
use projections::admin as proj_admin;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use crate::domain::jobs::snapshot_runner::{run_snapshot, SnapshotRunResult};

// ── Request body ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RebuildRequest {
    pub tenant_id: String,
    /// Start of rebuild range (inclusive), YYYY-MM-DD.
    pub from: NaiveDate,
    /// End of rebuild range (inclusive), YYYY-MM-DD.
    pub to: NaiveDate,
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
        return ApiError::new(403, "forbidden", "ADMIN_TOKEN is not configured; rebuild is disabled")
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

fn guard(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ApiError>)> {
    proj_admin::verify_admin_token(extract_token(headers)).map_err(|msg| {
        tracing::warn!(reason = msg, "Admin request rejected");
        (
            StatusCode::FORBIDDEN,
            Json(ApiError::new(403, "forbidden", msg)),
        )
    })
}

async fn projection_status(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<proj_admin::ProjectionStatusRequest>,
) -> Result<Json<proj_admin::ProjectionStatusResponse>, (StatusCode, Json<ApiError>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = proj_admin::query_projection_status(&pool, &req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::internal(e)),
            )
        })?;
    Ok(Json(resp))
}

async fn consistency_check(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<proj_admin::ConsistencyCheckRequest>,
) -> Result<Json<proj_admin::ConsistencyCheckResponse>, (StatusCode, Json<ApiError>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = proj_admin::query_consistency_check(&pool, &req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::internal(e)),
            )
        })?;
    Ok(Json(resp))
}

async fn list_projections(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<proj_admin::ProjectionListResponse>, (StatusCode, Json<ApiError>)> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = proj_admin::query_projection_list(&pool)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::internal(e)),
            )
        })?;
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
        .route("/api/reporting/admin/projections", get(list_projections))
        .with_state(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_admin_router_builds() {
        let pool = PgPool::connect_lazy("postgres://localhost/fake")
            .expect("connect_lazy for test");
        let _router = admin_router(pool);
    }
}

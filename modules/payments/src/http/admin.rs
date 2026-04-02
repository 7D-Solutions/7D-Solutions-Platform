//! Standardized admin endpoints for the Payments module.
//!
//! Endpoints (all require `X-Admin-Token` header):
//!   POST /api/payments/admin/projection-status
//!   POST /api/payments/admin/consistency-check
//!   GET  /api/payments/admin/projections

use crate::AppState;
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use platform_http_contracts::{ApiError, PaginatedResponse};
use projections::admin;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

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

impl From<admin::ProjectionStatusResponse> for ProjectionStatusSchema {
    fn from(r: admin::ProjectionStatusResponse) -> Self {
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

impl From<admin::ConsistencyCheckResponse> for ConsistencyCheckSchema {
    fn from(r: admin::ConsistencyCheckResponse) -> Self {
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

impl From<admin::ProjectionSummary> for ProjectionSummarySchema {
    fn from(s: admin::ProjectionSummary) -> Self {
        Self {
            projection_name: s.projection_name,
            tenant_count: s.tenant_count,
            total_events_processed: s.total_events_processed,
            last_updated: s.last_updated,
        }
    }
}

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-admin-token").and_then(|v| v.to_str().ok())
}

fn guard(headers: &HeaderMap) -> Result<(), ApiError> {
    admin::verify_admin_token(extract_token(headers)).map_err(|msg| {
        tracing::warn!(reason = msg, "Admin request rejected");
        ApiError::forbidden(msg)
    })
}

#[utoipa::path(
    post,
    path = "/api/payments/admin/projection-status",
    tag = "Admin",
    responses(
        (status = 200, description = "Projection cursor status", body = ProjectionStatusSchema),
        (status = 403, description = "Forbidden — invalid or missing admin token", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
)]
pub async fn projection_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<admin::ProjectionStatusRequest>,
) -> Result<Json<ProjectionStatusSchema>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = admin::query_projection_status(&state.pool, &req)
        .await
        .map_err(|e| {
            tracing::error!("Admin projection-status error: {}", e);
            ApiError::internal("Internal error")
        })?;
    Ok(Json(resp.into()))
}

#[utoipa::path(
    post,
    path = "/api/payments/admin/consistency-check",
    tag = "Admin",
    responses(
        (status = 200, description = "Consistency check result", body = ConsistencyCheckSchema),
        (status = 403, description = "Forbidden — invalid or missing admin token", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
)]
pub async fn consistency_check(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<admin::ConsistencyCheckRequest>,
) -> Result<Json<ConsistencyCheckSchema>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = admin::query_consistency_check(&state.pool, &req)
        .await
        .map_err(|e| {
            tracing::error!("Admin consistency-check error: {}", e);
            ApiError::internal("Internal error")
        })?;
    Ok(Json(resp.into()))
}

#[utoipa::path(
    get,
    path = "/api/payments/admin/projections",
    tag = "Admin",
    responses(
        (status = 200, description = "Paginated list of all known projections", body = PaginatedResponse<ProjectionSummarySchema>),
        (status = 403, description = "Forbidden — invalid or missing admin token", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
)]
pub async fn list_projections(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<PaginatedResponse<ProjectionSummarySchema>>, ApiError> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = admin::query_projection_list(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("Admin list-projections error: {}", e);
            ApiError::internal("Internal error")
        })?;
    let items: Vec<ProjectionSummarySchema> =
        resp.projections.into_iter().map(Into::into).collect();
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse::new(items, 1, total, total)))
}

/// Build the admin sub-router (state = Arc<AppState>).
pub fn admin_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/payments/admin/projection-status",
            post(projection_status),
        )
        .route(
            "/api/payments/admin/consistency-check",
            post(consistency_check),
        )
        .route("/api/payments/admin/projections", get(list_projections))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[tokio::test]
    async fn test_admin_router_builds() {
        let pool = PgPool::connect_lazy("postgres://localhost/fake").expect("test pool");
        let state = Arc::new(AppState {
            pool,
            tilled_api_key: None,
            tilled_account_id: None,
            tilled_webhook_secret: None,
            tilled_webhook_secret_prev: None,
        });
        let _router = admin_router(state);
    }
}

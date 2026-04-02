//! Standardized admin endpoints for the GL module.
//!
//! Endpoints (all require `X-Admin-Token` header):
//!   POST /api/gl/admin/projection-status
//!   POST /api/gl/admin/consistency-check
//!   GET  /api/gl/admin/projections

use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use platform_http_contracts::ApiError;
use projections::admin;
use sqlx::PgPool;

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
    path = "/api/gl/admin/projection-status",
    tag = "Admin",
    responses(
        (status = 200, description = "Projection cursor status for the requested projection"),
        (status = 403, description = "Invalid or missing admin token"),
    ),
)]
pub async fn projection_status(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<admin::ProjectionStatusRequest>,
) -> Result<Json<admin::ProjectionStatusResponse>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = admin::query_projection_status(&pool, &req)
        .await
        .map_err(|e| ApiError::internal(e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/api/gl/admin/consistency-check",
    tag = "Admin",
    responses(
        (status = 200, description = "Consistency digest for the requested projection table"),
        (status = 403, description = "Invalid or missing admin token"),
    ),
)]
pub async fn consistency_check(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<admin::ConsistencyCheckRequest>,
) -> Result<Json<admin::ConsistencyCheckResponse>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = admin::query_consistency_check(&pool, &req)
        .await
        .map_err(|e| ApiError::internal(e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    get,
    path = "/api/gl/admin/projections",
    tag = "Admin",
    responses(
        (status = 200, description = "List of all known projections with cursor summaries"),
        (status = 403, description = "Invalid or missing admin token"),
    ),
)]
pub async fn list_projections(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<admin::ProjectionListResponse>, ApiError> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = admin::query_projection_list(&pool)
        .await
        .map_err(|e| ApiError::internal(e))?;
    Ok(Json(resp))
}

/// Build the admin sub-router (state = PgPool).
pub fn admin_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/gl/admin/projection-status", post(projection_status))
        .route("/api/gl/admin/consistency-check", post(consistency_check))
        .route("/api/gl/admin/projections", get(list_projections))
        .with_state(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_admin_router_builds() {
        let pool = PgPool::connect_lazy("postgres://localhost/fake").expect("test pool");
        let _router = admin_router(pool);
    }
}

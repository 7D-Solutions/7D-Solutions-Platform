//! Standardized admin endpoints for the AR module.
//!
//! Endpoints (all require `X-Admin-Token` header):
//!   POST /api/ar/admin/projection-status
//!   POST /api/ar/admin/consistency-check
//!   GET  /api/ar/admin/projections

use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use projections::admin;
use sqlx::PgPool;

use crate::models::ApiError;

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-admin-token").and_then(|v| v.to_str().ok())
}

fn guard(headers: &HeaderMap) -> Result<(), ApiError> {
    admin::verify_admin_token(extract_token(headers)).map_err(|msg| {
        tracing::warn!(reason = msg, "Admin request rejected");
        ApiError::forbidden(msg)
    })
}

async fn projection_status(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<admin::ProjectionStatusRequest>,
) -> Result<Json<admin::ProjectionStatusResponse>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = admin::query_projection_status(&pool, &req)
        .await
        .map_err(|e| ApiError::internal(&e))?;
    Ok(Json(resp))
}

async fn consistency_check(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<admin::ConsistencyCheckRequest>,
) -> Result<Json<admin::ConsistencyCheckResponse>, ApiError> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = admin::query_consistency_check(&pool, &req)
        .await
        .map_err(|e| ApiError::internal(&e))?;
    Ok(Json(resp))
}

async fn list_projections(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<admin::ProjectionListResponse>, ApiError> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = admin::query_projection_list(&pool)
        .await
        .map_err(|e| ApiError::internal(&e))?;
    Ok(Json(resp))
}

/// Build the admin sub-router (state = PgPool).
pub fn admin_router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/ar/admin/projection-status", post(projection_status))
        .route("/api/ar/admin/consistency-check", post(consistency_check))
        .route("/api/ar/admin/projections", get(list_projections))
        .with_state(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_admin_router_builds() {
        // Verify the router compiles — no runtime DB needed
        let pool = PgPool::connect_lazy("postgres://localhost/fake").expect("test pool");
        let _router = admin_router(pool);
    }
}

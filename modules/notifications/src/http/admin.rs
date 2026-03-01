//! Standardized admin endpoints for the Notifications module.
//!
//! Endpoints (all require `X-Admin-Token` header):
//!   POST /api/notifications/admin/projection-status
//!   POST /api/notifications/admin/consistency-check
//!   GET  /api/notifications/admin/projections

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use projections::admin;
use sqlx::PgPool;

use super::admin_types::ErrorBody;

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-admin-token").and_then(|v| v.to_str().ok())
}

fn guard(headers: &HeaderMap) -> Result<(), (StatusCode, Json<ErrorBody>)> {
    admin::verify_admin_token(extract_token(headers)).map_err(|msg| {
        tracing::warn!(reason = msg, "Admin request rejected");
        (
            StatusCode::FORBIDDEN,
            Json(ErrorBody::new("forbidden", msg)),
        )
    })
}

async fn projection_status(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<admin::ProjectionStatusRequest>,
) -> Result<Json<admin::ProjectionStatusResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = admin::query_projection_status(&pool, &req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", &e)),
            )
        })?;
    Ok(Json(resp))
}

async fn consistency_check(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(req): Json<admin::ConsistencyCheckRequest>,
) -> Result<Json<admin::ConsistencyCheckResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = admin::query_consistency_check(&pool, &req)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", &e)),
            )
        })?;
    Ok(Json(resp))
}

async fn list_projections(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<admin::ProjectionListResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = admin::query_projection_list(&pool).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("internal_error", &e)),
        )
    })?;
    Ok(Json(resp))
}

/// Build the admin sub-router (state = PgPool).
pub fn admin_router(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/api/notifications/admin/projection-status",
            post(projection_status),
        )
        .route(
            "/api/notifications/admin/consistency-check",
            post(consistency_check),
        )
        .route(
            "/api/notifications/admin/projections",
            get(list_projections),
        )
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

//! Standardized admin endpoints for the Payments module.
//!
//! Endpoints (all require `X-Admin-Token` header):
//!   POST /api/payments/admin/projection-status
//!   POST /api/payments/admin/consistency-check
//!   GET  /api/payments/admin/projections

use crate::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use projections::admin;
use std::sync::Arc;

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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<admin::ProjectionStatusRequest>,
) -> Result<Json<admin::ProjectionStatusResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: projection-status");
    let resp = admin::query_projection_status(&state.pool, &req)
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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<admin::ConsistencyCheckRequest>,
) -> Result<Json<admin::ConsistencyCheckResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!(projection = %req.projection_name, "admin: consistency-check");
    let resp = admin::query_consistency_check(&state.pool, &req)
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
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<admin::ProjectionListResponse>, (StatusCode, Json<ErrorBody>)> {
    guard(&headers)?;
    tracing::info!("admin: list projections");
    let resp = admin::query_projection_list(&state.pool)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", &e)),
            )
        })?;
    Ok(Json(resp))
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
        let pool = PgPool::connect_lazy("postgres://localhost/fake").unwrap();
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

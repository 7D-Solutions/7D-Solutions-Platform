pub mod assets;

use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

/// GET /api/health — liveness probe (no external deps checked)
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fixed-assets",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(state): State<Arc<crate::AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Database not reachable: {}", e),
            )
        })?;

    Ok(Json(serde_json::json!({
        "status": "ready",
        "service": "fixed-assets",
        "database": "connected"
    })))
}

/// GET /api/version — module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "00000000000000";

    Json(serde_json::json!({
        "module_name": "fixed-assets",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

use axum::{extract::State, Json};
use std::sync::Arc;

/// Health check endpoint - returns basic service status (liveness)
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "inventory-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Readiness check endpoint - verifies DB connectivity
///
/// Returns 200 OK if ready, 503 Service Unavailable if DB is unreachable.
pub async fn ready(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    match sqlx::query("SELECT 1").execute(&app_state.pool).await {
        Ok(_) => Ok(Json(serde_json::json!({
            "status": "ready",
            "service": "inventory-rs",
            "database": "connected"
        }))),
        Err(e) => Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            format!("Database not reachable: {}", e),
        )),
    }
}

/// Version endpoint - returns module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260218000001";

    Json(serde_json::json!({
        "module_name": "inventory-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

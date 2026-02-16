use axum::{extract::State, Json};
use std::sync::Arc;

/// Health check endpoint - returns basic service status
///
/// This endpoint is for liveness checks. It does not verify external dependencies.
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "gl-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Readiness check endpoint - verifies DB connectivity
///
/// This endpoint is for readiness checks. It verifies that the service can:
/// - Connect to the database
/// - Execute queries
///
/// Returns 200 OK if ready, 503 Service Unavailable if not ready.
pub async fn ready(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    // Test DB connectivity with a simple query
    match sqlx::query("SELECT 1")
        .execute(&app_state.pool)
        .await
    {
        Ok(_) => Ok(Json(serde_json::json!({
            "status": "ready",
            "service": "gl-rs",
            "database": "connected"
        }))),
        Err(e) => Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            format!("Database not reachable: {}", e),
        )),
    }
}

/// Version endpoint - returns module identity and schema version
///
/// This endpoint provides build and deployment information:
/// - module_name: The service identifier
/// - module_version: Build version from Cargo.toml
/// - schema_version: Database schema version (latest migration)
///
/// Used for:
/// - Deployment verification
/// - Troubleshooting version mismatches
/// - Migration status checks
pub async fn version() -> Json<serde_json::Value> {
    // Schema version derived from latest migration timestamp
    // Format: YYYYMMDDNNNNNN (e.g., 20260216000001)
    const SCHEMA_VERSION: &str = "20260216000001";

    Json(serde_json::json!({
        "module_name": "gl-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

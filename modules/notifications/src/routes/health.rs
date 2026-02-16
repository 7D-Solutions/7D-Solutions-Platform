use axum::{extract::State, Json};
use sqlx::PgPool;

/// Health check endpoint - returns basic service status
///
/// This endpoint is for liveness checks. It does not verify external dependencies.
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "notifications-rs",
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
    State(pool): State<PgPool>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    // Test DB connectivity with a simple query
    match sqlx::query("SELECT 1")
        .execute(&pool)
        .await
    {
        Ok(_) => Ok(Json(serde_json::json!({
            "status": "ready",
            "service": "notifications-rs",
            "database": "connected"
        }))),
        Err(e) => Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            format!("Database not reachable: {}", e),
        )),
    }
}

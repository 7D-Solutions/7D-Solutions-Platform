use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

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
        "service": "consolidation",
        "database": "connected"
    })))
}

use axum::Json;

/// GET /api/health — liveness probe (no external deps checked)
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "ttp",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

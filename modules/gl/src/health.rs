use axum::Json;
use serde_json::Value;

/// Health check endpoint handler
pub async fn health() -> Json<Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "gl-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

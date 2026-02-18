use axum::Json;

/// GET /api/version — module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module_name": "consolidation",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": "none"
    }))
}

use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check, healthz as healthz_helper, ready_response_to_axum,
    HealthzResponse, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;

/// GET /healthz — standardized liveness probe
pub async fn healthz() -> Json<HealthzResponse> {
    healthz_helper().await
}

/// GET /api/health — legacy liveness probe
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "shipping-receiving-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(state): State<Arc<crate::AppState>>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let resp = build_ready_response(
        "shipping-receiving",
        env!("CARGO_PKG_VERSION"),
        vec![db_check(latency, db_err)],
    );
    ready_response_to_axum(resp)
}

/// GET /api/version — module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260225000001";

    Json(serde_json::json!({
        "module_name": "shipping-receiving-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

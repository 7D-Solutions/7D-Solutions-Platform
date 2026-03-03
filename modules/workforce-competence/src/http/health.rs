use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "workforce-competence-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn ready(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&app_state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let pool_metrics = PoolMetrics {
        size: app_state.pool.size(),
        idle: app_state.pool.num_idle() as u32,
        active: app_state
            .pool
            .size()
            .saturating_sub(app_state.pool.num_idle() as u32),
    };

    let resp = build_ready_response(
        "workforce-competence",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260303000001";

    Json(serde_json::json!({
        "module_name": "workforce-competence-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

pub async fn schema_version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260303000001";

    Json(serde_json::json!({
        "schema_version": SCHEMA_VERSION
    }))
}

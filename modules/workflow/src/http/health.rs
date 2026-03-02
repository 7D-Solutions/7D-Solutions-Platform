//! Health and readiness endpoints for the Workflow module.

use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "workflow",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

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

    let pool_metrics = PoolMetrics {
        size: state.pool.size(),
        idle: state.pool.num_idle() as u32,
        active: state
            .pool
            .size()
            .saturating_sub(state.pool.num_idle() as u32),
    };

    let resp = build_ready_response(
        "workflow",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260302000001";

    Json(serde_json::json!({
        "module_name": "workflow",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

use axum::{extract::State, http::StatusCode, Json};
use health::{build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse};
use sqlx::PgPool;
use std::time::Instant;

/// Health check endpoint - returns basic service status
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "pdf-editor-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(pool): State<PgPool>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let pool_metrics = PoolMetrics {
        size: pool.size(),
        idle: pool.num_idle() as u32,
        active: pool.size().saturating_sub(pool.num_idle() as u32),
    };

    let resp = build_ready_response(
        "pdf-editor",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

/// Version endpoint
pub async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module_name": "pdf-editor-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
    }))
}

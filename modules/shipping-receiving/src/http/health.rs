use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, healthz as healthz_helper, ready_response_to_axum,
    HealthzResponse, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;

/// GET /healthz — standardized liveness probe
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "Health",
    responses(
        (status = 200, description = "Liveness check — service process is up"),
    ),
)]
pub async fn healthz() -> Json<HealthzResponse> {
    healthz_helper().await
}

/// GET /api/health — legacy liveness probe
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    responses(
        (status = 200, description = "Legacy liveness check — service is healthy"),
    ),
)]
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "shipping-receiving-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
#[utoipa::path(
    get,
    path = "/api/ready",
    tag = "Health",
    responses(
        (status = 200, description = "All dependency checks passed — service is ready"),
        (status = 503, description = "One or more dependency checks failed"),
    ),
)]
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
        "shipping-receiving",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

/// GET /api/version — module identity and schema version
#[utoipa::path(
    get,
    path = "/api/version",
    tag = "Health",
    responses(
        (status = 200, description = "Module name, version, and schema version"),
    ),
)]
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260225000001";

    Json(serde_json::json!({
        "module_name": "shipping-receiving-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

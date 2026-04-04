use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;

#[utoipa::path(
    get,
    path = "/healthz",
    tag = "Health",
    responses(
        (status = 200, description = "Liveness check — service process is up"),
    ),
)]
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "gl-rs",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

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
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = crate::repos::checklist_repo::ping(&app_state.pool)
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
        "gl",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

#[utoipa::path(
    get,
    path = "/api/version",
    tag = "Health",
    responses(
        (status = 200, description = "Module name, version, and schema version"),
    ),
)]
pub async fn version() -> Json<serde_json::Value> {
    // Schema version derived from latest migration timestamp
    // Format: YYYYMMDDNNNNNN (e.g., 20260216000001)
    const SCHEMA_VERSION: &str = "20260216000001";

    Json(serde_json::json!({
        "module_name": "gl-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

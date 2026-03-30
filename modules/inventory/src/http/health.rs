use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, nats_check, ready_response_to_axum, PoolMetrics,
    ReadyResponse, ReadyStatus,
};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Health check endpoint - returns basic service status (legacy, kept for compat)
pub async fn health() -> Json<serde_json::Value> {
    let uptime = START_TIME.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "healthy",
        "service": "inventory-rs",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
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

    let db_ok = db_err.is_none();
    let resp = {
        let checks = vec![
            db_check_with_pool(latency, db_err.clone(), pool_metrics),
            nats_check(app_state.bus_health.is_connected(), app_state.bus_health.latency_ms()),
        ];
        build_ready_response("inventory", env!("CARGO_PKG_VERSION"), checks)
    };

    let resp = if resp.status == ReadyStatus::Down && db_ok {
        ReadyResponse {
            status: ReadyStatus::Degraded,
            degraded: true,
            ..resp
        }
    } else {
        resp
    };

    ready_response_to_axum(resp)
}

/// Version endpoint - returns module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260218000001";

    Json(serde_json::json!({
        "module_name": "inventory-rs",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}

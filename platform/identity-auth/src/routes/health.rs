use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct HealthState {
    pub db: PgPool,
    pub nats: async_nats::Client,
    pub metrics: Metrics,
}

/// GET /health/live — legacy liveness probe (kept for backward compatibility)
pub async fn health_live() -> StatusCode {
    StatusCode::OK
}

/// GET /health/ready — legacy readiness probe (kept for backward compatibility)
pub async fn health_ready(
    State(state): State<Arc<HealthState>>,
) -> Result<Json<Value>, StatusCode> {
    // DB check
    let db_ok = sqlx::query("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();

    state
        .metrics
        .dep_up
        .with_label_values(&["db"])
        .set(if db_ok { 1 } else { 0 });

    if !db_ok {
        state.metrics.dep_up.with_label_values(&["ready"]).set(0);
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // NATS check
    let nats_ok = state.nats.connection_state() == async_nats::connection::State::Connected;
    state
        .metrics
        .dep_up
        .with_label_values(&["nats"])
        .set(if nats_ok { 1 } else { 0 });

    if !nats_ok {
        state.metrics.dep_up.with_label_values(&["ready"]).set(0);
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    state.metrics.dep_up.with_label_values(&["ready"]).set(1);

    Ok(Json(json!({
        "status": "ready",
        "database": "connected",
        "nats": "connected"
    })))
}

/// GET /api/ready — standardized readiness probe (platform health contract)
pub async fn ready(
    State(state): State<Arc<HealthState>>,
) -> Result<Json<health::ReadyResponse>, (StatusCode, Json<health::ReadyResponse>)> {
    let start = std::time::Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .err()
        .map(|e| e.to_string());
    let db_latency = start.elapsed().as_millis() as u64;

    state
        .metrics
        .dep_up
        .with_label_values(&["db"])
        .set(if db_err.is_none() { 1 } else { 0 });

    let nats_ok = state.nats.connection_state() == async_nats::connection::State::Connected;
    state
        .metrics
        .dep_up
        .with_label_values(&["nats"])
        .set(if nats_ok { 1 } else { 0 });

    let pool_metrics = health::PoolMetrics {
        size: state.db.size(),
        idle: state.db.num_idle() as u32,
        active: state.db.size().saturating_sub(state.db.num_idle() as u32),
    };

    let resp = health::build_ready_response(
        "identity-auth",
        env!("CARGO_PKG_VERSION"),
        vec![
            health::db_check_with_pool(db_latency, db_err, pool_metrics),
            health::nats_check(nats_ok, 0),
        ],
    );

    state.metrics.dep_up.with_label_values(&["ready"]).set(
        if resp.status == health::ReadyStatus::Ready { 1 } else { 0 },
    );

    health::ready_response_to_axum(resp)
}

pub mod handoffs;
pub mod holds;
pub mod labels;
pub mod signoffs;
pub mod verifications;

use axum::extract::State;
use health::{build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics};
use std::sync::Arc;
use std::time::Instant;

use crate::AppState;

pub async fn health_check(State(state): State<Arc<AppState>>) -> impl axum::response::IntoResponse {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;
    let pool_metrics = PoolMetrics {
        size: state.pool.size(),
        idle: state.pool.num_idle() as u32,
        active: state.pool.size().saturating_sub(state.pool.num_idle() as u32),
    };
    let resp = build_ready_response(
        "shop-floor-gates",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

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

pub async fn health_live() -> StatusCode {
    StatusCode::OK
}

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

use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct HealthState {
    pub db: PgPool,
    pub nats: async_nats::Client,
}

pub async fn health_live() -> StatusCode {
    StatusCode::OK
}

pub async fn health_ready(
    State(state): State<Arc<HealthState>>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("SELECT 1")
        .fetch_one(&state.db)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    if state.nats.connection_state() != async_nats::connection::State::Connected {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(json!({
        "status": "ready",
        "database": "connected",
        "nats": "connected"
    })))
}

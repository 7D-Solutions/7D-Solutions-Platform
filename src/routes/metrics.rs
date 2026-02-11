use axum::{extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct MetricsState {
    pub metrics: Metrics,
}

pub async fn metrics(State(state): State<Arc<MetricsState>>) -> impl IntoResponse {
    match state.metrics.render() {
        Ok(body) => (StatusCode::OK, body),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("metrics error: {e}")),
    }
}

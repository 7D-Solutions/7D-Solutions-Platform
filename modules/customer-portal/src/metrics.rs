use axum::{http::StatusCode, response::IntoResponse};
use prometheus::{Encoder, Registry, TextEncoder};

#[derive(Clone)]
pub struct PortalMetrics {
    pub registry: Registry,
}

impl PortalMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        Ok(Self {
            registry: Registry::new(),
        })
    }
}

pub async fn metrics_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::AppState>>,
) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = state.metrics.registry.gather();
    let mut buffer = Vec::new();

    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => (StatusCode::OK, String::from_utf8_lossy(&buffer).to_string()).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to encode metrics".to_string(),
        )
            .into_response(),
    }
}

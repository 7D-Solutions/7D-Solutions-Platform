//! Prometheus metrics for the Numbering service.

use axum::http::StatusCode;
use prometheus::{IntCounterVec, Opts, Registry};

pub struct NumberingMetrics {
    pub allocations_total: IntCounterVec,
    pub replays_total: IntCounterVec,
    registry: Registry,
}

impl NumberingMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let allocations_total = IntCounterVec::new(
            Opts::new(
                "numbering_allocations_total",
                "Total number allocations by entity",
            ),
            &["entity"],
        )?;
        registry.register(Box::new(allocations_total.clone()))?;

        let replays_total = IntCounterVec::new(
            Opts::new(
                "numbering_replays_total",
                "Total idempotency replays by entity",
            ),
            &["entity"],
        )?;
        registry.register(Box::new(replays_total.clone()))?;

        Ok(Self {
            allocations_total,
            replays_total,
            registry,
        })
    }
}

pub async fn metrics_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&state.metrics.registry.gather(), &mut buffer)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("metrics encode error: {}", e),
            )
        })?;
    String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("metrics utf8 error: {}", e),
        )
    })
}

use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use std::sync::Arc;

/// Consolidation-specific Prometheus metrics
pub struct ConsolidationMetrics {
    pub consolidation_runs_total: IntCounter,
    registry: Registry,
}

impl ConsolidationMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let consolidation_runs_total = IntCounter::new(
            "consolidation_runs_total",
            "Total number of consolidation runs executed",
        )?;
        registry.register(Box::new(consolidation_runs_total.clone()))?;

        Ok(Self {
            consolidation_runs_total,
            registry,
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// Axum handler for GET /metrics
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e),
        )
    })?;
    String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to convert metrics to UTF-8: {}", e),
        )
    })
}

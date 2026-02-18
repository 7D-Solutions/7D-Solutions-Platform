use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use std::sync::Arc;

/// AP-specific Prometheus metrics
pub struct ApMetrics {
    pub bills_created_total: IntCounter,
    pub bills_approved_total: IntCounter,
    pub payments_initiated_total: IntCounter,
    registry: Registry,
}

impl ApMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let bills_created_total =
            IntCounter::new("ap_bills_created_total", "Total AP bills created")?;
        registry.register(Box::new(bills_created_total.clone()))?;

        let bills_approved_total =
            IntCounter::new("ap_bills_approved_total", "Total AP bills approved")?;
        registry.register(Box::new(bills_approved_total.clone()))?;

        let payments_initiated_total = IntCounter::new(
            "ap_payments_initiated_total",
            "Total AP payments initiated",
        )?;
        registry.register(Box::new(payments_initiated_total.clone()))?;

        Ok(Self {
            bills_created_total,
            bills_approved_total,
            payments_initiated_total,
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

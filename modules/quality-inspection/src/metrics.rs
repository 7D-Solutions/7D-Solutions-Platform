use axum::{extract::State, http::StatusCode};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct QualityInspectionMetrics {
    pub inspection_operations_total: IntCounter,
    pub http_request_duration_seconds: HistogramVec,
    pub http_requests_total: IntCounterVec,
    registry: Registry,
}

impl QualityInspectionMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let inspection_operations_total = IntCounter::new(
            "quality_inspection_operations_total",
            "Total number of quality inspection operations processed",
        )?;
        registry.register(Box::new(inspection_operations_total.clone()))?;

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "quality_inspection_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        let http_requests_total = IntCounterVec::new(
            Opts::new(
                "quality_inspection_http_requests_total",
                "Total HTTP requests",
            ),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        Ok(Self {
            inspection_operations_total,
            http_request_duration_seconds,
            http_requests_total,
            registry,
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();

    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e),
        )
    })?;

    String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to convert metrics to string: {}", e),
        )
    })
}

//! Prometheus metrics for the Workflow module.

use axum::{extract::State, http::StatusCode, Json};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

use crate::routes::ErrorBody;

pub struct WorkflowMetrics {
    pub http_request_duration_seconds: HistogramVec,
    pub http_requests_total: IntCounterVec,
    pub outbox_queue_depth: IntGauge,
    pub events_enqueued_total: IntCounter,
    registry: Registry,
}

impl WorkflowMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "workflow_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        let http_requests_total = IntCounterVec::new(
            Opts::new("workflow_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        let outbox_queue_depth = IntGauge::new(
            "workflow_outbox_queue_depth",
            "Number of unpublished events in outbox",
        )?;
        registry.register(Box::new(outbox_queue_depth.clone()))?;

        let events_enqueued_total = IntCounter::new(
            "workflow_events_enqueued_total",
            "Total events enqueued to outbox (all time)",
        )?;
        registry.register(Box::new(events_enqueued_total.clone()))?;

        Ok(Self {
            http_request_duration_seconds,
            http_requests_total,
            outbox_queue_depth,
            events_enqueued_total,
            registry,
        })
    }

    pub fn record_http_request(&self, method: &str, route: &str, status: &str, duration_secs: f64) {
        self.http_request_duration_seconds
            .with_label_values(&[method, route, status])
            .observe(duration_secs);
        self.http_requests_total
            .with_label_values(&[method, route, status])
            .inc();
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

pub async fn metrics_handler(
    State(state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match crate::outbox::count_unpublished(&state.pool).await {
        Ok(depth) => state.metrics.outbox_queue_depth.set(depth),
        Err(e) => tracing::warn!("Failed to fetch outbox queue depth: {}", e),
    }

    let encoder = TextEncoder::new();
    let families = state.metrics.registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new(
                "internal_error",
                &format!("Failed to encode metrics: {}", e),
            )),
        )
    })?;
    String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new(
                "internal_error",
                &format!("Failed to convert metrics to UTF-8: {}", e),
            )),
        )
    })
}

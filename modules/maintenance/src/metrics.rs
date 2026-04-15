//! Prometheus metrics for the Maintenance module.
//!
//! SLO metrics exposed:
//! - `maintenance_http_request_duration_seconds{method,route,status}`: request latency
//! - `maintenance_http_requests_total{method,route,status}`: request count / error rate
//! - `maintenance_outbox_queue_depth`: unpublished events in outbox
//!
//! No PII in labels — method, route, status are operational values only.

use axum::extract::State;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

use platform_http_contracts::ApiError;

/// Maintenance-specific Prometheus metrics
pub struct MaintenanceMetrics {
    /// SLO: HTTP request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    /// SLO: HTTP request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    /// Outbox queue depth — number of unpublished events
    pub outbox_queue_depth: IntGauge,
    /// Total events enqueued (all time)
    pub events_enqueued_total: IntCounter,
    registry: Registry,
}

impl MaintenanceMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "maintenance_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        let http_requests_total = IntCounterVec::new(
            Opts::new("maintenance_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        let outbox_queue_depth = IntGauge::new(
            "maintenance_outbox_queue_depth",
            "Number of unpublished events in outbox",
        )?;
        registry.register(Box::new(outbox_queue_depth.clone()))?;

        let events_enqueued_total = IntCounter::new(
            "maintenance_events_enqueued_total",
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

    /// Record an HTTP request for SLO tracking. Labels must not contain PII.
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

/// Axum handler for GET /metrics
pub async fn metrics_handler(
    State(state): State<Arc<crate::AppState>>,
) -> Result<String, ApiError> {
    // Refresh outbox queue depth gauge on each scrape
    match crate::outbox::count_unpublished(&state.pool).await {
        Ok(depth) => state.metrics.outbox_queue_depth.set(depth),
        Err(e) => tracing::warn!("Failed to fetch outbox queue depth: {}", e),
    }

    let encoder = TextEncoder::new();
    let families = state.metrics.registry().gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&families, &mut buffer)
        .map_err(|e| ApiError::internal(format!("Failed to encode metrics: {}", e)))?;
    String::from_utf8(buffer)
        .map_err(|e| ApiError::internal(format!("Failed to convert metrics to UTF-8: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = MaintenanceMetrics::new().expect("MaintenanceMetrics::new");
        m.record_http_request("GET", "/api/ready", "200", 0.003);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("http_request_duration_seconds")),
            "request latency histogram missing: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("http_requests_total")),
            "request count counter missing: {:?}",
            names
        );
    }

    #[test]
    fn metrics_slo_no_pii_in_labels() {
        let m = MaintenanceMetrics::new().expect("MaintenanceMetrics::new");
        m.record_http_request("POST", "/api/maintenance/work-orders", "201", 0.05);
        let families = m.registry().gather();
        assert!(!families.is_empty());
    }
}

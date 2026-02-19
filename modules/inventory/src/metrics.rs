//! Prometheus metrics for Inventory module
//!
//! Metrics exposed:
//! - inventory_operations_total: Total count of inventory operations
//!
//! Design principle: Metrics must not mask errors or miscount operations.
//! All counters are append-only and never decrease.

use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec,
                 Opts, Registry, TextEncoder};
use std::sync::Arc;

/// Inventory metrics registry
#[derive(Clone)]
pub struct InventoryMetrics {
    pub inventory_operations_total: IntCounter,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    registry: Registry,
}

impl InventoryMetrics {
    /// Create new Inventory metrics registry
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let inventory_operations_total = IntCounter::new(
            "inventory_operations_total",
            "Total number of inventory operations processed",
        )?;
        registry.register(Box::new(inventory_operations_total.clone()))?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "inventory_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter
        let http_requests_total = IntCounterVec::new(
            Opts::new("inventory_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new("inventory_event_consumer_lag_messages", "Event consumer lag in messages"),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            inventory_operations_total,
            http_request_duration_seconds,
            http_requests_total,
            event_consumer_lag_messages,
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

    /// Record event consumer lag.
    pub fn record_consumer_lag(&self, consumer_group: &str, lag: i64) {
        self.event_consumer_lag_messages
            .with_label_values(&[consumer_group])
            .set(lag);
    }

    /// Get registry for /metrics endpoint
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = InventoryMetrics::new().expect("InventoryMetrics::new");
        m.record_http_request("GET", "/api/inventory/items", "200", 0.012);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("http_request_duration_seconds")),
            "request latency histogram missing: {:?}", names
        );
        assert!(
            names.iter().any(|n| n.contains("http_requests_total")),
            "request count counter missing: {:?}", names
        );
    }

    #[test]
    fn metrics_slo_exports_consumer_lag() {
        let m = InventoryMetrics::new().expect("InventoryMetrics::new");
        m.record_consumer_lag("inventory_consumer", 7);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("event_consumer_lag_messages")),
            "consumer lag metric missing: {:?}", names
        );
    }
}

/// Axum handler for /metrics endpoint
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();

    let mut buffer = vec![];
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| {
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

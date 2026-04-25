//! Prometheus metrics for Shipping-Receiving module
//!
//! Metrics exposed:
//! - shipping_receiving_operations_total: Total count of shipping/receiving operations
//! - shipping_receiving_http_request_duration_seconds: HTTP request latency histogram
//! - shipping_receiving_http_requests_total: Total HTTP requests by method/route/status
//! - shipping_receiving_event_consumer_lag_messages: Event consumer lag gauge

use axum::extract::State;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

use platform_http_contracts::ApiError;

#[derive(Clone)]
pub struct ShippingReceivingMetrics {
    pub shipping_receiving_operations_total: IntCounter,
    pub http_request_duration_seconds: HistogramVec,
    pub http_requests_total: IntCounterVec,
    pub event_consumer_lag_messages: IntGaugeVec,
    /// label_reprint_total{carrier, result} — result is ok|carrier_not_found|carrier_error
    pub label_reprint_total: IntCounterVec,
    registry: Registry,
}

impl ShippingReceivingMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let shipping_receiving_operations_total = IntCounter::new(
            "shipping_receiving_operations_total",
            "Total number of shipping/receiving operations processed",
        )?;
        registry.register(Box::new(shipping_receiving_operations_total.clone()))?;

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "shipping_receiving_http_request_duration_seconds",
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
                "shipping_receiving_http_requests_total",
                "Total HTTP requests",
            ),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new(
                "shipping_receiving_event_consumer_lag_messages",
                "Event consumer lag in messages",
            ),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        let label_reprint_total = IntCounterVec::new(
            Opts::new(
                "shipping_receiving_label_reprint_total",
                "Total label reprint requests by carrier and result",
            ),
            &["carrier", "result"],
        )?;
        registry.register(Box::new(label_reprint_total.clone()))?;

        Ok(Self {
            shipping_receiving_operations_total,
            http_request_duration_seconds,
            http_requests_total,
            event_consumer_lag_messages,
            label_reprint_total,
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

    pub fn record_consumer_lag(&self, consumer_group: &str, lag: i64) {
        self.event_consumer_lag_messages
            .with_label_values(&[consumer_group])
            .set(lag);
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = ShippingReceivingMetrics::new().expect("ShippingReceivingMetrics::new");
        m.record_http_request("GET", "/api/shipping-receiving/shipments", "200", 0.012);
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
    fn metrics_slo_exports_consumer_lag() {
        let m = ShippingReceivingMetrics::new().expect("ShippingReceivingMetrics::new");
        m.record_consumer_lag("shipping_receiving_consumer", 7);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("event_consumer_lag_messages")),
            "consumer lag metric missing: {:?}",
            names
        );
    }

    #[test]
    fn metrics_exports_operations_total() {
        let m = ShippingReceivingMetrics::new().expect("ShippingReceivingMetrics::new");
        m.shipping_receiving_operations_total.inc();
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("operations_total")),
            "operations_total counter missing: {:?}",
            names
        );
    }

    #[test]
    fn metrics_all_five_families_present() {
        let m = ShippingReceivingMetrics::new().expect("ShippingReceivingMetrics::new");
        // Trigger all metrics so they appear in gather
        m.shipping_receiving_operations_total.inc();
        m.record_http_request("GET", "/test", "200", 0.01);
        m.record_consumer_lag("test_group", 0);
        m.label_reprint_total
            .with_label_values(&["ups", "ok"])
            .inc();
        let families = m.registry().gather();
        assert_eq!(
            families.len(),
            5,
            "Expected 5 metric families, got {}",
            families.len()
        );
    }
}

/// Axum handler for /metrics endpoint
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, ApiError> {
    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();

    let mut buffer = vec![];
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| ApiError::internal(format!("Failed to encode metrics: {}", e)))?;

    String::from_utf8(buffer)
        .map_err(|e| ApiError::internal(format!("Failed to convert metrics to string: {}", e)))
}

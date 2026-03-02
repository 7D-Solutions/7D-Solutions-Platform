//! Prometheus metrics for Subscriptions module
//!
//! This module provides operational observability for the Subscriptions service,
//! exposing metrics about billing cycles and subscription lifecycle.

use axum::{extract::State, http::StatusCode};
use projections::metrics::ProjectionMetrics;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

/// Subscriptions-specific metrics registry
pub struct SubscriptionsMetrics {
    pub cycles_attempted_total: IntCounter,
    pub cycles_completed_total: IntCounter,
    pub subscription_churn_total: IntCounter,
    pub projection_metrics: ProjectionMetrics,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    registry: Registry,
}

impl SubscriptionsMetrics {
    /// Create a new metrics registry with Subscriptions-specific metrics
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Counter: Total billing cycles attempted
        let cycles_attempted_total = IntCounter::new(
            "subscriptions_cycles_attempted_total",
            "Total number of billing cycles attempted",
        )?;
        registry.register(Box::new(cycles_attempted_total.clone()))?;

        // Counter: Total billing cycles completed successfully
        let cycles_completed_total = IntCounter::new(
            "subscriptions_cycles_completed_total",
            "Total number of billing cycles completed successfully",
        )?;
        registry.register(Box::new(cycles_completed_total.clone()))?;

        // Counter: Total subscription cancellations (churn)
        let subscription_churn_total = IntCounter::new(
            "subscriptions_churn_total",
            "Total number of subscription cancellations",
        )?;
        registry.register(Box::new(subscription_churn_total.clone()))?;

        // Initialize projection metrics
        let projection_metrics = ProjectionMetrics::new()?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "subscriptions_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter
        let http_requests_total = IntCounterVec::new(
            Opts::new("subscriptions_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new(
                "subscriptions_event_consumer_lag_messages",
                "Event consumer lag in messages",
            ),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            cycles_attempted_total,
            cycles_completed_total,
            subscription_churn_total,
            projection_metrics,
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

    /// Get the underlying registry (for gathering metrics)
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = SubscriptionsMetrics::new().expect("SubscriptionsMetrics::new");
        m.record_http_request("GET", "/api/subscriptions", "200", 0.025);
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
        let m = SubscriptionsMetrics::new().expect("SubscriptionsMetrics::new");
        m.record_consumer_lag("subscriptions_consumer", 2);
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
}

/// Axum handler for /metrics endpoint
///
/// Returns Prometheus-formatted metrics in text/plain format
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();

    // Gather metrics from both Subscriptions metrics and projection metrics
    let mut metric_families = app_state.metrics.registry().gather();
    let projection_metric_families = app_state.metrics.projection_metrics.registry().gather();
    metric_families.extend(projection_metric_families);

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

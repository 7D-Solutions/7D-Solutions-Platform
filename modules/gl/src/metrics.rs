//! Prometheus metrics for GL module
//!
//! Phase 16: Operational observability for financial operations
//!
//! Metrics exposed:
//! - journal_entries_total: Total count of journal entries created
//! - posting_errors_total: Total count of posting errors encountered
//!
//! Design principle: Metrics must not mask errors or miscount entries.
//! All counters are append-only and never decrease.

use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGaugeVec,
                 Opts, Registry, TextEncoder};
use projections::metrics::ProjectionMetrics;
use std::sync::Arc;
use axum::{extract::State, http::StatusCode};

/// GL metrics registry
#[derive(Clone)]
pub struct GlMetrics {
    pub journal_entries_total: IntCounter,
    pub posting_errors_total: IntCounter,
    pub projection_metrics: ProjectionMetrics,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    registry: Registry,
}

impl GlMetrics {
    /// Create new GL metrics registry
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let journal_entries_total = IntCounter::new(
            "gl_journal_entries_total",
            "Total number of journal entries created"
        )?;

        let posting_errors_total = IntCounter::new(
            "gl_posting_errors_total",
            "Total number of posting errors encountered"
        )?;

        registry.register(Box::new(journal_entries_total.clone()))?;
        registry.register(Box::new(posting_errors_total.clone()))?;

        // Initialize projection metrics
        let projection_metrics = ProjectionMetrics::new()
            .map_err(|e| prometheus::Error::Msg(format!("Failed to create projection metrics: {}", e)))?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "gl_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "route", "status"],
        )
        .map_err(|e| prometheus::Error::Msg(e.to_string()))?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter
        let http_requests_total = IntCounterVec::new(
            Opts::new("gl_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )
        .map_err(|e| prometheus::Error::Msg(e.to_string()))?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new("gl_event_consumer_lag_messages", "Event consumer lag in messages"),
            &["consumer_group"],
        )
        .map_err(|e| prometheus::Error::Msg(e.to_string()))?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            journal_entries_total,
            posting_errors_total,
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
        let m = GlMetrics::new().expect("GlMetrics::new");
        m.record_http_request("POST", "/api/gl/journal-entries", "201", 0.03);
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
        let m = GlMetrics::new().expect("GlMetrics::new");
        m.record_consumer_lag("gl_consumer", 10);
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

    // Gather metrics from both GL metrics and projection metrics
    let mut metric_families = app_state.metrics.registry().gather();
    let projection_metric_families = app_state.metrics.projection_metrics.registry().gather();
    metric_families.extend(projection_metric_families);

    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer)
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e)
        ))?;

    String::from_utf8(buffer)
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to convert metrics to string: {}", e)
        ))
}

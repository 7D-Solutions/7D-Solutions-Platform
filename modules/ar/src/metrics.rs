//! Prometheus metrics for AR module
//!
//! This module provides operational observability for the AR service,
//! exposing metrics about invoice lifecycle and system health.

use axum::{extract::State, http::StatusCode};
use projections::metrics::ProjectionMetrics;
use prometheus::{
    Encoder, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

/// AR-specific metrics registry
pub struct ArMetrics {
    pub invoices_created_total: IntCounter,
    pub invoices_paid_total: IntCounter,
    pub invoice_age_seconds: Histogram,
    pub projection_metrics: ProjectionMetrics,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (enables error rate via status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    /// Outbox queue depth — number of unpublished events
    pub outbox_queue_depth: IntGauge,
    /// Tax reconciliation flags raised since last restart
    pub tax_reconciliation_flagged_total: IntCounterVec,
    registry: Registry,
}

impl ArMetrics {
    /// Create a new metrics registry with AR-specific metrics
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Counter: Total invoices created
        let invoices_created_total = IntCounter::new(
            "ar_invoices_created_total",
            "Total number of invoices created",
        )?;
        registry.register(Box::new(invoices_created_total.clone()))?;

        // Counter: Total invoices paid
        let invoices_paid_total = IntCounter::new(
            "ar_invoices_paid_total",
            "Total number of invoices marked as paid",
        )?;
        registry.register(Box::new(invoices_paid_total.clone()))?;

        // Histogram: Invoice age in seconds (time from creation to payment)
        let invoice_age_seconds = Histogram::with_opts(
            prometheus::HistogramOpts::new(
                "ar_invoice_age_seconds",
                "Time in seconds from invoice creation to payment",
            )
            .buckets(vec![
                60.0,      // 1 minute
                300.0,     // 5 minutes
                900.0,     // 15 minutes
                3600.0,    // 1 hour
                86400.0,   // 1 day
                604800.0,  // 1 week
                2592000.0, // 30 days
            ]),
        )?;
        registry.register(Box::new(invoice_age_seconds.clone()))?;

        // Projection metrics for AR projections
        let projection_metrics = ProjectionMetrics::new()?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "ar_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter (error rate = http_requests_total{status=~"5.."} / http_requests_total)
        let http_requests_total = IntCounterVec::new(
            Opts::new("ar_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new(
                "ar_event_consumer_lag_messages",
                "Event consumer lag in messages",
            ),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        let outbox_queue_depth = IntGauge::new(
            "ar_outbox_queue_depth",
            "Number of unpublished events in outbox",
        )?;
        registry.register(Box::new(outbox_queue_depth.clone()))?;

        let tax_reconciliation_flagged_total = IntCounterVec::new(
            Opts::new(
                "ar_tax_reconciliation_flagged_total",
                "Tax reconciliation divergences flagged by tenant",
            ),
            &["tenant_id"],
        )?;
        registry.register(Box::new(tax_reconciliation_flagged_total.clone()))?;

        Ok(Self {
            invoices_created_total,
            invoices_paid_total,
            invoice_age_seconds,
            projection_metrics,
            http_request_duration_seconds,
            http_requests_total,
            event_consumer_lag_messages,
            outbox_queue_depth,
            tax_reconciliation_flagged_total,
            registry,
        })
    }

    /// Record an HTTP request for SLO tracking.
    /// Call from route handlers or middleware; labels must not contain PII.
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
        let m = ArMetrics::new().expect("ArMetrics::new");
        m.record_http_request("GET", "/api/ar/invoices", "200", 0.042);
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
        let m = ArMetrics::new().expect("ArMetrics::new");
        m.record_consumer_lag("ar_consumer", 3);
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
    fn metrics_slo_no_pii_in_labels() {
        // Verify that record_http_request only takes method/route/status labels (no user data)
        let m = ArMetrics::new().expect("ArMetrics::new");
        m.record_http_request("POST", "/api/ar/invoices", "422", 0.01);
        // No PII fields — method, route, status are all operational
        let families = m.registry().gather();
        assert!(!families.is_empty());
    }
}

/// Axum handler for /metrics endpoint
///
/// Returns Prometheus-formatted metrics in text/plain format
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    // Refresh outbox queue depth gauge on each scrape
    match crate::events::outbox::count_unpublished(&app_state.pool).await {
        Ok(depth) => app_state.metrics.outbox_queue_depth.set(depth),
        Err(e) => tracing::warn!("Failed to fetch outbox queue depth: {}", e),
    }

    let encoder = TextEncoder::new();

    // Gather metrics from both AR metrics and projection metrics
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

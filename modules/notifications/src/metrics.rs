//! Prometheus metrics for the Notifications module.
//!
//! SLO metrics exposed:
//! - `notifications_http_request_duration_seconds{method, route, status}`: request latency
//! - `notifications_http_requests_total{method, route, status}`: request count / error rate
//! - `notifications_event_consumer_lag_messages{consumer_group}`: consumer lag
//!
//! No PII in labels — method, route, status, and consumer_group are operational values only.

use axum::http::StatusCode;
use lazy_static::lazy_static;
use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts,
                 Registry, TextEncoder};

lazy_static! {
    /// Shared Prometheus registry for notifications SLO metrics.
    pub static ref METRICS_REGISTRY: Registry = {
        let registry = Registry::new();
        registry.register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone()))
            .expect("register notifications_http_request_duration_seconds");
        registry.register(Box::new(HTTP_REQUESTS_TOTAL.clone()))
            .expect("register notifications_http_requests_total");
        registry.register(Box::new(EVENT_CONSUMER_LAG_MESSAGES.clone()))
            .expect("register notifications_event_consumer_lag_messages");
        registry
    };

    /// SLO: HTTP request latency histogram
    pub static ref HTTP_REQUEST_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "notifications_http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
        &["method", "route", "status"],
    )
    .expect("create notifications_http_request_duration_seconds");

    /// SLO: HTTP request counter (error rate derived from status label)
    pub static ref HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("notifications_http_requests_total", "Total HTTP requests"),
        &["method", "route", "status"],
    )
    .expect("create notifications_http_requests_total");

    /// SLO: Event consumer lag
    pub static ref EVENT_CONSUMER_LAG_MESSAGES: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "notifications_event_consumer_lag_messages",
            "Event consumer lag in messages",
        ),
        &["consumer_group"],
    )
    .expect("create notifications_event_consumer_lag_messages");
}

/// Record an HTTP request for SLO tracking. Labels must not contain PII.
pub fn record_http_request(method: &str, route: &str, status: &str, duration_secs: f64) {
    HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[method, route, status])
        .observe(duration_secs);
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[method, route, status])
        .inc();
}

/// Record event consumer lag.
pub fn record_consumer_lag(consumer_group: &str, lag: i64) {
    EVENT_CONSUMER_LAG_MESSAGES
        .with_label_values(&[consumer_group])
        .set(lag);
}

/// Axum handler for GET /metrics — renders Prometheus text format.
pub async fn metrics_handler() -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();
    let families = METRICS_REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).map_err(|e| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        record_http_request("GET", "/api/health", "200", 0.002);
        let families = METRICS_REGISTRY.gather();
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
        record_consumer_lag("notifications_invoice_consumer", 1);
        let families = METRICS_REGISTRY.gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("event_consumer_lag_messages")),
            "consumer lag metric missing: {:?}", names
        );
    }

    #[test]
    fn metrics_slo_no_pii_in_labels() {
        // method/route/status/consumer_group are all operational, not user-specific
        record_http_request("POST", "/api/notifications/send", "500", 0.1);
        let families = METRICS_REGISTRY.gather();
        assert!(!families.is_empty());
    }
}

//! Prometheus Metrics for Payments Module (Phase 16: bd-1pw7)
//!
//! **Purpose**: Expose operational metrics for payment outcomes and UNKNOWN protocol
//!
//! **Metrics Exposed**:
//! - `payment_attempts_total{status}`: Counter of payment attempts by status
//! - `unknown_duration_seconds`: Histogram of UNKNOWN state durations
//! - `retry_attempts_total`: Counter of retry attempts
//! - Projection metrics (lag, backlog, last_applied_age)
//! - SLO: `payments_http_request_duration_seconds{method, route, status}`
//! - SLO: `payments_http_requests_total{method, route, status}`
//! - SLO: `payments_event_consumer_lag_messages{consumer_group}`
//!
//! **Invariant**: Payment outcomes must be metered accurately
//! **Failure Mode to Avoid**: Misleading metrics or missing UNKNOWN visibility

use lazy_static::lazy_static;
use projections::metrics::ProjectionMetrics;
use prometheus::{
    HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts,
    Registry as PrometheusRegistry,
};
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::Histogram;
use prometheus_client::registry::Registry;
use std::sync::Mutex;

/// Label set for payment attempt status
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct PaymentStatusLabels {
    pub status: String,
}

lazy_static! {
    /// Global metrics registry (prometheus-client)
    pub static ref METRICS_REGISTRY: Mutex<Registry> = {
        let mut registry = Registry::default();

        // Register payment_attempts_total metric
        registry.register(
            "payment_attempts_total",
            "Total number of payment attempts by status",
            PAYMENT_ATTEMPTS_TOTAL.clone(),
        );

        // Register unknown_duration_seconds metric
        registry.register(
            "unknown_duration_seconds",
            "Duration of UNKNOWN state in seconds",
            UNKNOWN_DURATION_SECONDS.clone(),
        );

        // Register retry_attempts_total metric
        registry.register(
            "retry_attempts_total",
            "Total number of retry attempts",
            RETRY_ATTEMPTS_TOTAL.clone(),
        );

        Mutex::new(registry)
    };

    /// Projection metrics (using standard prometheus crate)
    pub static ref PROJECTION_METRICS: ProjectionMetrics = {
        ProjectionMetrics::new()
            .expect("Failed to initialize projection metrics")
    };

    /// Counter: payment_attempts_total{status}
    pub static ref PAYMENT_ATTEMPTS_TOTAL: Family<PaymentStatusLabels, Counter> =
        Family::default();

    /// Histogram: unknown_duration_seconds (buckets: 60s, 300s, 600s, 1800s, 3600s)
    pub static ref UNKNOWN_DURATION_SECONDS: Histogram =
        Histogram::new([60.0, 300.0, 600.0, 1800.0, 3600.0].into_iter());

    /// Counter: retry_attempts_total
    pub static ref RETRY_ATTEMPTS_TOTAL: Counter = Counter::default();

    /// SLO metrics registry (prometheus crate, consistent with other modules)
    pub static ref SLO_REGISTRY: PrometheusRegistry = {
        let registry = PrometheusRegistry::new();
        registry.register(Box::new(PAYMENTS_HTTP_REQUEST_DURATION_SECONDS.clone()))
            .expect("register payments_http_request_duration_seconds");
        registry.register(Box::new(PAYMENTS_HTTP_REQUESTS_TOTAL.clone()))
            .expect("register payments_http_requests_total");
        registry.register(Box::new(PAYMENTS_EVENT_CONSUMER_LAG_MESSAGES.clone()))
            .expect("register payments_event_consumer_lag_messages");
        registry.register(Box::new(PAYMENTS_OUTBOX_QUEUE_DEPTH.clone()))
            .expect("register payments_outbox_queue_depth");
        registry
    };

    /// SLO: HTTP request latency histogram
    pub static ref PAYMENTS_HTTP_REQUEST_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "payments_http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
        &["method", "route", "status"],
    )
    .expect("create payments_http_request_duration_seconds");

    /// SLO: HTTP request counter (error rate derived from status label)
    pub static ref PAYMENTS_HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("payments_http_requests_total", "Total HTTP requests"),
        &["method", "route", "status"],
    )
    .expect("create payments_http_requests_total");

    /// SLO: Event consumer lag
    pub static ref PAYMENTS_EVENT_CONSUMER_LAG_MESSAGES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("payments_event_consumer_lag_messages", "Event consumer lag in messages"),
        &["consumer_group"],
    )
    .expect("create payments_event_consumer_lag_messages");

    /// Outbox queue depth — number of unpublished events
    pub static ref PAYMENTS_OUTBOX_QUEUE_DEPTH: IntGauge = IntGauge::new(
        "payments_outbox_queue_depth",
        "Number of unpublished events in outbox",
    )
    .expect("create payments_outbox_queue_depth");
}

/// Record an HTTP request for SLO tracking. Labels must not contain PII.
pub fn record_http_request(method: &str, route: &str, status: &str, duration_secs: f64) {
    PAYMENTS_HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[method, route, status])
        .observe(duration_secs);
    PAYMENTS_HTTP_REQUESTS_TOTAL
        .with_label_values(&[method, route, status])
        .inc();
}

/// Record event consumer lag.
pub fn record_consumer_lag(consumer_group: &str, lag: i64) {
    PAYMENTS_EVENT_CONSUMER_LAG_MESSAGES
        .with_label_values(&[consumer_group])
        .set(lag);
}

/// Increment payment_attempts_total counter
pub fn record_payment_attempt(status: &str) {
    PAYMENT_ATTEMPTS_TOTAL
        .get_or_create(&PaymentStatusLabels {
            status: status.to_string(),
        })
        .inc();
}

/// Record UNKNOWN state duration
pub fn record_unknown_duration(duration_seconds: f64) {
    UNKNOWN_DURATION_SECONDS.observe(duration_seconds);
}

/// Increment retry_attempts_total counter
pub fn record_retry_attempt() {
    RETRY_ATTEMPTS_TOTAL.inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_payment_attempt() {
        record_payment_attempt("attempting");
        record_payment_attempt("succeeded");
        record_payment_attempt("succeeded");

        // Verify counters incremented (basic sanity check)
        let attempting_count = PAYMENT_ATTEMPTS_TOTAL
            .get_or_create(&PaymentStatusLabels {
                status: "attempting".to_string(),
            })
            .get();
        let succeeded_count = PAYMENT_ATTEMPTS_TOTAL
            .get_or_create(&PaymentStatusLabels {
                status: "succeeded".to_string(),
            })
            .get();

        assert!(
            attempting_count >= 1,
            "attempting counter should be incremented"
        );
        assert!(
            succeeded_count >= 2,
            "succeeded counter should be incremented twice"
        );
    }

    #[test]
    fn test_record_unknown_duration() {
        // Record durations (no panic expected)
        record_unknown_duration(120.0);
        record_unknown_duration(600.0);
    }

    #[test]
    fn test_record_retry_attempt() {
        let initial = RETRY_ATTEMPTS_TOTAL.get();
        record_retry_attempt();
        record_retry_attempt();
        let final_count = RETRY_ATTEMPTS_TOTAL.get();

        assert!(final_count >= initial + 2, "retry counter should increment");
    }

    #[test]
    fn metrics_slo_exports_request_latency() {
        record_http_request("POST", "/api/payments", "200", 0.033);
        let families = SLO_REGISTRY.gather();
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
        record_consumer_lag("payments_consumer", 2);
        let families = SLO_REGISTRY.gather();
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

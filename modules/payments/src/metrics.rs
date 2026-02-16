//! Prometheus Metrics for Payments Module (Phase 16: bd-1pw7)
//!
//! **Purpose**: Expose operational metrics for payment outcomes and UNKNOWN protocol
//!
//! **Metrics Exposed**:
//! - `payment_attempts_total{status}`: Counter of payment attempts by status
//! - `unknown_duration_seconds`: Histogram of UNKNOWN state durations
//! - `retry_attempts_total`: Counter of retry attempts
//!
//! **Invariant**: Payment outcomes must be metered accurately
//! **Failure Mode to Avoid**: Misleading metrics or missing UNKNOWN visibility

use lazy_static::lazy_static;
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
    /// Global metrics registry
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

    /// Counter: payment_attempts_total{status}
    pub static ref PAYMENT_ATTEMPTS_TOTAL: Family<PaymentStatusLabels, Counter> =
        Family::default();

    /// Histogram: unknown_duration_seconds (buckets: 60s, 300s, 600s, 1800s, 3600s)
    pub static ref UNKNOWN_DURATION_SECONDS: Histogram =
        Histogram::new([60.0, 300.0, 600.0, 1800.0, 3600.0].into_iter());

    /// Counter: retry_attempts_total
    pub static ref RETRY_ATTEMPTS_TOTAL: Counter = Counter::default();
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

        assert!(attempting_count >= 1, "attempting counter should be incremented");
        assert!(succeeded_count >= 2, "succeeded counter should be incremented twice");
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
}

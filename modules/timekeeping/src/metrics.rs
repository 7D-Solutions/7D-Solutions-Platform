use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
                 IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::Arc;

/// Timekeeping Prometheus metrics
pub struct TimekeepingMetrics {
    pub time_entries_created_total: IntCounter,
    pub approvals_total: IntCounter,
    pub exports_total: IntCounter,
    pub active_timesheets_count: IntGauge,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    registry: Registry,
}

impl TimekeepingMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let time_entries_created_total = IntCounter::new(
            "tk_time_entries_created_total",
            "Total time entries created",
        )?;
        registry.register(Box::new(time_entries_created_total.clone()))?;

        let approvals_total =
            IntCounter::new("tk_approvals_total", "Total timesheet approvals processed")?;
        registry.register(Box::new(approvals_total.clone()))?;

        let exports_total =
            IntCounter::new("tk_exports_total", "Total timesheet export runs")?;
        registry.register(Box::new(exports_total.clone()))?;

        let active_timesheets_count = IntGauge::new(
            "tk_active_timesheets_count",
            "Current count of active (open) timesheets",
        )?;
        registry.register(Box::new(active_timesheets_count.clone()))?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "tk_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter
        let http_requests_total = IntCounterVec::new(
            Opts::new("tk_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new("tk_event_consumer_lag_messages", "Event consumer lag in messages"),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            time_entries_created_total,
            approvals_total,
            exports_total,
            active_timesheets_count,
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

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = TimekeepingMetrics::new().expect("TimekeepingMetrics::new");
        m.record_http_request("GET", "/api/timekeeping/entries", "200", 0.008);
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
        let m = TimekeepingMetrics::new().expect("TimekeepingMetrics::new");
        m.record_consumer_lag("tk_consumer", 6);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("event_consumer_lag_messages")),
            "consumer lag metric missing: {:?}", names
        );
    }
}

/// Axum handler for GET /metrics
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();
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

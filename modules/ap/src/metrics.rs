use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
                 IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::Arc;

/// AP-specific Prometheus metrics
pub struct ApMetrics {
    pub bills_created_total: IntCounter,
    pub bills_approved_total: IntCounter,
    pub payments_initiated_total: IntCounter,
    /// Current count of open (unpaid/non-voided) bills — updated on each scrape.
    pub open_bills_count: IntGauge,
    /// Current count of open bills past their due_date — updated on each scrape.
    pub overdue_bills_count: IntGauge,
    /// Sum of total_minor for all open bills — updated on each scrape.
    pub total_open_amount_minor: IntGauge,
    /// Total payment runs created (all time) — updated on each scrape.
    pub payment_runs_created: IntGauge,
    /// Total allocations created (all time) — updated on each scrape.
    pub allocations_created: IntGauge,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    registry: Registry,
}

impl ApMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let bills_created_total =
            IntCounter::new("ap_bills_created_total", "Total AP bills created")?;
        registry.register(Box::new(bills_created_total.clone()))?;

        let bills_approved_total =
            IntCounter::new("ap_bills_approved_total", "Total AP bills approved")?;
        registry.register(Box::new(bills_approved_total.clone()))?;

        let payments_initiated_total = IntCounter::new(
            "ap_payments_initiated_total",
            "Total AP payments initiated",
        )?;
        registry.register(Box::new(payments_initiated_total.clone()))?;

        let open_bills_count =
            IntGauge::new("ap_open_bills_count", "Current count of open (unpaid) AP bills")?;
        registry.register(Box::new(open_bills_count.clone()))?;

        let overdue_bills_count = IntGauge::new(
            "ap_overdue_bills_count",
            "Current count of AP bills past their due date",
        )?;
        registry.register(Box::new(overdue_bills_count.clone()))?;

        let total_open_amount_minor = IntGauge::new(
            "ap_total_open_amount_minor",
            "Sum of total_minor for all open (unpaid) AP bills",
        )?;
        registry.register(Box::new(total_open_amount_minor.clone()))?;

        let payment_runs_created = IntGauge::new(
            "ap_payment_runs_created",
            "Total AP payment runs created (all time)",
        )?;
        registry.register(Box::new(payment_runs_created.clone()))?;

        let allocations_created = IntGauge::new(
            "ap_allocations_created",
            "Total AP payment allocations created (all time)",
        )?;
        registry.register(Box::new(allocations_created.clone()))?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "ap_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter
        let http_requests_total = IntCounterVec::new(
            Opts::new("ap_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new("ap_event_consumer_lag_messages", "Event consumer lag in messages"),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            bills_created_total,
            bills_approved_total,
            payments_initiated_total,
            open_bills_count,
            overdue_bills_count,
            total_open_amount_minor,
            payment_runs_created,
            allocations_created,
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
        let m = ApMetrics::new().expect("ApMetrics::new");
        m.record_http_request("GET", "/api/ap/bills", "200", 0.018);
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
        let m = ApMetrics::new().expect("ApMetrics::new");
        m.record_consumer_lag("ap_consumer", 4);
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
    // Refresh operational gauges from DB on each scrape.
    match crate::domain::reports::metrics::fetch_snapshot(&app_state.pool).await {
        Ok(snapshot) => {
            let m = &app_state.metrics;
            m.open_bills_count.set(snapshot.open_bills_count);
            m.overdue_bills_count.set(snapshot.overdue_bills_count);
            m.total_open_amount_minor
                .set(snapshot.total_open_amount_minor);
            m.payment_runs_created.set(snapshot.payment_runs_created);
            m.allocations_created.set(snapshot.allocations_created);
        }
        Err(e) => {
            // Log and continue — stale gauge values are preferable to a failed scrape.
            tracing::warn!("Failed to fetch AP metrics snapshot: {}", e);
        }
    }

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

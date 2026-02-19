//! Prometheus metrics for the Treasury module.
//!
//! Exposes operational counters (import success/fail), recon gauges
//! (match rate, unmatched counts), and endpoint latency histograms.
//! Gauges are refreshed from the database on each `/metrics` scrape.
//! No PII appears in any metric label.

use axum::{extract::State, http::StatusCode};
use prometheus::{
    Encoder, Gauge, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

use crate::domain::recon::metrics as recon_metrics;

/// Treasury-specific Prometheus metrics.
pub struct TreasuryMetrics {
    // -- Existing counters --
    pub accounts_created_total: IntCounter,
    pub transactions_recorded_total: IntCounter,
    pub statements_imported_total: IntCounter,

    // -- Existing gauges --
    pub open_transactions_count: IntGauge,
    pub accounts_count: IntGauge,

    // -- Import ops --
    pub import_success_total: IntCounter,
    pub import_fail_total: IntCounter,

    // -- Recon gauges (refreshed from DB on scrape) --
    pub recon_matched_total: IntGauge,
    pub recon_unmatched_lines: IntGauge,
    pub recon_unmatched_txns: IntGauge,
    pub recon_match_rate: Gauge,

    // -- Endpoint latency --
    pub endpoint_latency: HistogramVec,

    // SLO: standardized request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,

    registry: Registry,
}

impl TreasuryMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Existing counters
        let accounts_created_total = IntCounter::new(
            "treasury_accounts_created_total",
            "Total treasury accounts created",
        )?;
        registry.register(Box::new(accounts_created_total.clone()))?;

        let transactions_recorded_total = IntCounter::new(
            "treasury_transactions_recorded_total",
            "Total treasury transactions recorded",
        )?;
        registry.register(Box::new(transactions_recorded_total.clone()))?;

        let statements_imported_total = IntCounter::new(
            "treasury_statements_imported_total",
            "Total bank statements imported",
        )?;
        registry.register(Box::new(statements_imported_total.clone()))?;

        // Existing gauges
        let open_transactions_count = IntGauge::new(
            "treasury_open_transactions_count",
            "Current count of unreconciled treasury transactions",
        )?;
        registry.register(Box::new(open_transactions_count.clone()))?;

        let accounts_count = IntGauge::new(
            "treasury_accounts_count",
            "Current count of treasury accounts",
        )?;
        registry.register(Box::new(accounts_count.clone()))?;

        // Import ops
        let import_success_total = IntCounter::new(
            "treasury_import_success_total",
            "Successful statement imports",
        )?;
        registry.register(Box::new(import_success_total.clone()))?;

        let import_fail_total = IntCounter::new(
            "treasury_import_fail_total",
            "Failed statement imports",
        )?;
        registry.register(Box::new(import_fail_total.clone()))?;

        // Recon gauges
        let recon_matched_total = IntGauge::new(
            "treasury_recon_matched_total",
            "Active (non-superseded) recon matches",
        )?;
        registry.register(Box::new(recon_matched_total.clone()))?;

        let recon_unmatched_lines = IntGauge::new(
            "treasury_recon_unmatched_lines",
            "Unmatched imported statement lines",
        )?;
        registry.register(Box::new(recon_unmatched_lines.clone()))?;

        let recon_unmatched_txns = IntGauge::new(
            "treasury_recon_unmatched_txns",
            "Unmatched payment transactions",
        )?;
        registry.register(Box::new(recon_unmatched_txns.clone()))?;

        let recon_match_rate = Gauge::new(
            "treasury_recon_match_rate",
            "Ratio of matched lines to total imported lines (0.0-1.0)",
        )?;
        registry.register(Box::new(recon_match_rate.clone()))?;

        // Endpoint latency histogram
        let latency_opts = HistogramOpts::new(
            "treasury_http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]);
        let endpoint_latency = HistogramVec::new(latency_opts, &["method", "endpoint"])?;
        registry.register(Box::new(endpoint_latency.clone()))?;

        // SLO: request counter (method/route/status for error rate)
        let http_requests_total = IntCounterVec::new(
            Opts::new("treasury_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new("treasury_event_consumer_lag_messages", "Event consumer lag in messages"),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            accounts_created_total,
            transactions_recorded_total,
            statements_imported_total,
            open_transactions_count,
            accounts_count,
            import_success_total,
            import_fail_total,
            recon_matched_total,
            recon_unmatched_lines,
            recon_unmatched_txns,
            recon_match_rate,
            endpoint_latency,
            http_requests_total,
            event_consumer_lag_messages,
            registry,
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Convenience: record a successful import.
    pub fn record_import_success(&self) {
        self.import_success_total.inc();
        self.statements_imported_total.inc();
    }

    /// Convenience: record a failed import.
    pub fn record_import_fail(&self) {
        self.import_fail_total.inc();
    }

    /// Record an HTTP request for SLO tracking. Labels must not contain PII.
    pub fn record_http_request(&self, method: &str, route: &str, status: &str, duration_secs: f64) {
        self.endpoint_latency
            .with_label_values(&[method, route])
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
}

/// Axum handler for GET /metrics.
///
/// Refreshes DB-backed gauges (recon stats, account/txn counts) before
/// encoding, so every Prometheus scrape sees current values.
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    // Refresh recon gauges from DB
    if let Ok(snap) = recon_metrics::snapshot(&app_state.pool).await {
        app_state.metrics.recon_matched_total.set(snap.matched);
        app_state
            .metrics
            .recon_unmatched_lines
            .set(snap.unmatched_lines);
        app_state
            .metrics
            .recon_unmatched_txns
            .set(snap.unmatched_txns);
        app_state.metrics.recon_match_rate.set(snap.match_rate);
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

/// Axum middleware that records per-endpoint latency.
///
/// UUIDs in the path are replaced with `:id` to keep label cardinality low.
pub async fn latency_layer(
    State(state): State<Arc<crate::AppState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let start = std::time::Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    let response = next.run(req).await;

    let duration = start.elapsed().as_secs_f64();
    let sanitized = sanitize_path(&path);
    state
        .metrics
        .endpoint_latency
        .with_label_values(&[&method, &sanitized])
        .observe(duration);

    response
}

/// Replace UUID path segments with `:id` to avoid high-cardinality labels.
fn sanitize_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            if uuid::Uuid::parse_str(seg).is_ok() {
                ":id"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_registry_creates_successfully() {
        let m = TreasuryMetrics::new().expect("metrics init");
        // HistogramVec only appears in gather() after at least one observation.
        m.endpoint_latency
            .with_label_values(&["GET", "/test"])
            .observe(0.001);
        // All metrics should be registered — gathering should not panic.
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();

        assert!(names.contains(&"treasury_import_success_total"));
        assert!(names.contains(&"treasury_import_fail_total"));
        assert!(names.contains(&"treasury_recon_match_rate"));
        assert!(names.contains(&"treasury_recon_unmatched_lines"));
        assert!(names.contains(&"treasury_recon_unmatched_txns"));
        assert!(names.contains(&"treasury_http_request_duration_seconds"));
    }

    #[test]
    fn sanitize_path_replaces_uuids() {
        let path = "/api/treasury/accounts/550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(sanitize_path(path), "/api/treasury/accounts/:id");
    }

    #[test]
    fn sanitize_path_preserves_non_uuid_segments() {
        let path = "/api/treasury/recon/auto-match";
        assert_eq!(sanitize_path(path), "/api/treasury/recon/auto-match");
    }

    #[test]
    fn record_import_counters() {
        let m = TreasuryMetrics::new().expect("metrics init");
        m.record_import_success();
        m.record_import_success();
        m.record_import_fail();

        assert_eq!(m.import_success_total.get(), 2);
        assert_eq!(m.import_fail_total.get(), 1);
        assert_eq!(m.statements_imported_total.get(), 2);
    }

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = TreasuryMetrics::new().expect("metrics init");
        m.record_http_request("GET", "/api/treasury/accounts", "200", 0.022);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("http_request_duration_seconds")),
            "request latency histogram missing: {:?}", names
        );
        assert!(
            names.iter().any(|n| n.contains("treasury_http_requests_total")),
            "request count counter missing: {:?}", names
        );
    }

    #[test]
    fn metrics_slo_exports_consumer_lag() {
        let m = TreasuryMetrics::new().expect("metrics init");
        m.record_consumer_lag("treasury_payments_consumer", 0);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("event_consumer_lag_messages")),
            "consumer lag metric missing: {:?}", names
        );
    }
}

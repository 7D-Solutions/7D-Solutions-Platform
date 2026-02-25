use axum::{extract::State, http::StatusCode, Json};
use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
                 IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::Arc;

use crate::http::admin_types::ErrorBody;

/// Fixed Assets Prometheus metrics
pub struct FixedAssetsMetrics {
    pub assets_created_total: IntCounter,
    pub depreciation_runs_total: IntCounter,
    pub disposals_total: IntCounter,
    /// Current count of active assets
    pub active_assets_count: IntGauge,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag
    pub event_consumer_lag_messages: IntGaugeVec,
    registry: Registry,
}

impl FixedAssetsMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let assets_created_total = IntCounter::new(
            "fa_assets_created_total",
            "Total fixed assets created",
        )?;
        registry.register(Box::new(assets_created_total.clone()))?;

        let depreciation_runs_total = IntCounter::new(
            "fa_depreciation_runs_total",
            "Total depreciation runs executed",
        )?;
        registry.register(Box::new(depreciation_runs_total.clone()))?;

        let disposals_total = IntCounter::new(
            "fa_disposals_total",
            "Total asset disposals recorded",
        )?;
        registry.register(Box::new(disposals_total.clone()))?;

        let active_assets_count = IntGauge::new(
            "fa_active_assets_count",
            "Current count of active fixed assets",
        )?;
        registry.register(Box::new(active_assets_count.clone()))?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "fa_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        // SLO: request counter
        let http_requests_total = IntCounterVec::new(
            Opts::new("fa_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: event consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new("fa_event_consumer_lag_messages", "Event consumer lag in messages"),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        Ok(Self {
            assets_created_total,
            depreciation_runs_total,
            disposals_total,
            active_assets_count,
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
        let m = FixedAssetsMetrics::new().expect("FixedAssetsMetrics::new");
        m.record_http_request("GET", "/api/fixed-assets", "200", 0.015);
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
        let m = FixedAssetsMetrics::new().expect("FixedAssetsMetrics::new");
        m.record_consumer_lag("fa_consumer", 1);
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
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    let _ = &app_state.metrics;

    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("internal_error", &format!("Failed to encode metrics: {}", e))),
        )
    })?;
    String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("internal_error", &format!("Failed to convert metrics to UTF-8: {}", e))),
        )
    })
}

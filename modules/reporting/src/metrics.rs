use axum::{extract::State, http::StatusCode};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts,
    Registry, TextEncoder,
};
use std::sync::Arc;

/// Reporting-specific Prometheus metrics
pub struct ReportingMetrics {
    pub queries_executed_total: IntCounter,
    pub cache_hits_total: IntCounter,
    pub cache_misses_total: IntCounter,
    /// Number of cached KPI results currently stored.
    pub kpi_cache_size: IntGauge,
    /// Number of ingestion checkpoints tracked.
    pub ingestion_checkpoints: IntGauge,
    /// Row counts per cache table (label: table).
    pub cache_rows_total: IntGaugeVec,
    // SLO: request latency histogram
    pub http_request_duration_seconds: HistogramVec,
    // SLO: request counter (error rate derived from status label)
    pub http_requests_total: IntCounterVec,
    // SLO: event consumer lag (ingest pipeline)
    pub event_consumer_lag_messages: IntGaugeVec,
    /// Ingestion processing time by stream.
    pub ingestion_lag_seconds: HistogramVec,
    registry: Registry,
}

impl ReportingMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let queries_executed_total = IntCounter::new(
            "reporting_queries_executed_total",
            "Total reporting queries executed",
        )?;
        registry.register(Box::new(queries_executed_total.clone()))?;

        let cache_hits_total =
            IntCounter::new("reporting_cache_hits_total", "Total reporting cache hits")?;
        registry.register(Box::new(cache_hits_total.clone()))?;

        let cache_misses_total = IntCounter::new(
            "reporting_cache_misses_total",
            "Total reporting cache misses",
        )?;
        registry.register(Box::new(cache_misses_total.clone()))?;

        let kpi_cache_size = IntGauge::new(
            "reporting_kpi_cache_size",
            "Number of cached KPI results currently stored",
        )?;
        registry.register(Box::new(kpi_cache_size.clone()))?;

        let ingestion_checkpoints = IntGauge::new(
            "reporting_ingestion_checkpoints",
            "Number of ingestion checkpoints tracked",
        )?;
        registry.register(Box::new(ingestion_checkpoints.clone()))?;

        let cache_rows_total = IntGaugeVec::new(
            Opts::new(
                "reporting_cache_rows_total",
                "Row count per reporting cache table",
            ),
            &["table"],
        )?;
        registry.register(Box::new(cache_rows_total.clone()))?;

        // SLO: request latency
        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "reporting_http_request_duration_seconds",
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
            Opts::new("reporting_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        // SLO: ingest pipeline consumer lag
        let event_consumer_lag_messages = IntGaugeVec::new(
            Opts::new(
                "reporting_event_consumer_lag_messages",
                "Event consumer lag in messages",
            ),
            &["consumer_group"],
        )?;
        registry.register(Box::new(event_consumer_lag_messages.clone()))?;

        // Ingestion processing time per stream
        let ingestion_lag_seconds = HistogramVec::new(
            HistogramOpts::new(
                "reporting_ingestion_lag_seconds",
                "Time to process one ingestion event per stream",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]),
            &["stream"],
        )?;
        registry.register(Box::new(ingestion_lag_seconds.clone()))?;

        Ok(Self {
            queries_executed_total,
            cache_hits_total,
            cache_misses_total,
            kpi_cache_size,
            ingestion_checkpoints,
            cache_rows_total,
            http_request_duration_seconds,
            http_requests_total,
            event_consumer_lag_messages,
            ingestion_lag_seconds,
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

    /// Record ingest pipeline consumer lag.
    pub fn record_consumer_lag(&self, consumer_group: &str, lag: i64) {
        self.event_consumer_lag_messages
            .with_label_values(&[consumer_group])
            .set(lag);
    }

    /// Record ingestion processing time for a stream.
    pub fn record_ingestion_lag(&self, stream: &str, duration_secs: f64) {
        self.ingestion_lag_seconds
            .with_label_values(&[stream])
            .observe(duration_secs);
    }

    /// Update cache row count gauge for a specific table.
    pub fn set_cache_rows(&self, table: &str, count: i64) {
        self.cache_rows_total.with_label_values(&[table]).set(count);
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// Axum handler for GET /metrics
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    // Refresh operational gauges from DB on each scrape.
    if let Err(e) = refresh_gauges(&app_state).await {
        tracing::warn!("Failed to refresh reporting metrics gauges: {}", e);
        // Continue — stale gauge values are preferable to a failed scrape.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_slo_exports_request_latency() {
        let m = ReportingMetrics::new().expect("ReportingMetrics::new");
        m.record_http_request("GET", "/api/reporting/kpis", "200", 0.045);
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
        let m = ReportingMetrics::new().expect("ReportingMetrics::new");
        m.record_consumer_lag("reporting_ingest", 8);
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
    fn metrics_exports_ingestion_lag_seconds() {
        let m = ReportingMetrics::new().expect("ReportingMetrics::new");
        m.record_ingestion_lag("ar.events.ar.ar_aging_updated", 0.012);
        m.record_ingestion_lag("ap.events.ap.vendor_bill_created", 0.008);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("ingestion_lag_seconds")),
            "ingestion_lag_seconds histogram missing: {:?}",
            names
        );
    }

    #[test]
    fn metrics_exports_cache_rows_total() {
        let m = ReportingMetrics::new().expect("ReportingMetrics::new");
        m.set_cache_rows("rpt_ar_aging_cache", 42);
        m.set_cache_rows("rpt_ap_aging_cache", 18);
        m.set_cache_rows("rpt_kpi_cache", 7);
        let families = m.registry().gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.iter().any(|n| n.contains("cache_rows_total")),
            "cache_rows_total gauge missing: {:?}",
            names
        );
    }
}

// ── Gauge refresh ─────────────────────────────────────────────────────────────

const CACHE_TABLES: &[&str] = &[
    "rpt_ar_aging_cache",
    "rpt_ap_aging_cache",
    "rpt_kpi_cache",
    "rpt_trial_balance_cache",
    "rpt_cashflow_cache",
    "rpt_statement_cache",
];

async fn refresh_gauges(app_state: &Arc<crate::AppState>) -> Result<(), sqlx::Error> {
    let kpi_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rpt_kpi_cache")
        .fetch_optional(&app_state.pool)
        .await
        .unwrap_or(None)
        .unwrap_or(0);

    let checkpoint_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rpt_ingestion_checkpoints")
            .fetch_optional(&app_state.pool)
            .await
            .unwrap_or(None)
            .unwrap_or(0);

    app_state.metrics.kpi_cache_size.set(kpi_count);
    app_state
        .metrics
        .ingestion_checkpoints
        .set(checkpoint_count);

    // Per-table cache row counts
    for &table in CACHE_TABLES {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", table))
            .fetch_optional(&app_state.pool)
            .await
            .unwrap_or(None)
            .unwrap_or(0);
        app_state.metrics.set_cache_rows(table, count);
    }

    Ok(())
}

use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
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

        let cache_hits_total = IntCounter::new(
            "reporting_cache_hits_total",
            "Total reporting cache hits",
        )?;
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

        Ok(Self {
            queries_executed_total,
            cache_hits_total,
            cache_misses_total,
            kpi_cache_size,
            ingestion_checkpoints,
            registry,
        })
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

async fn refresh_gauges(app_state: &Arc<crate::AppState>) -> Result<(), sqlx::Error> {
    let kpi_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reporting_kpi_cache WHERE expires_at > NOW()",
    )
    .fetch_optional(&app_state.pool)
    .await
    .unwrap_or(None)
    .unwrap_or(0);

    let checkpoint_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reporting_ingestion_checkpoints")
            .fetch_optional(&app_state.pool)
            .await
            .unwrap_or(None)
            .unwrap_or(0);

    app_state.metrics.kpi_cache_size.set(kpi_count);
    app_state
        .metrics
        .ingestion_checkpoints
        .set(checkpoint_count);

    Ok(())
}

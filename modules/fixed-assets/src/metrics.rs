use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::Arc;

/// Fixed Assets Prometheus metrics
pub struct FixedAssetsMetrics {
    pub assets_created_total: IntCounter,
    pub depreciation_runs_total: IntCounter,
    pub disposals_total: IntCounter,
    /// Current count of active assets
    pub active_assets_count: IntGauge,
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

        Ok(Self {
            assets_created_total,
            depreciation_runs_total,
            disposals_total,
            active_assets_count,
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
    let _ = &app_state.metrics;

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

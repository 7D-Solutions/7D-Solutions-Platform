use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::Arc;

/// Timekeeping Prometheus metrics
pub struct TimekeepingMetrics {
    pub time_entries_created_total: IntCounter,
    pub approvals_total: IntCounter,
    pub exports_total: IntCounter,
    pub active_timesheets_count: IntGauge,
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

        Ok(Self {
            time_entries_created_total,
            approvals_total,
            exports_total,
            active_timesheets_count,
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

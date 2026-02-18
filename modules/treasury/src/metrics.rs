use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::Arc;

/// Treasury-specific Prometheus metrics
pub struct TreasuryMetrics {
    pub accounts_created_total: IntCounter,
    pub transactions_recorded_total: IntCounter,
    pub statements_imported_total: IntCounter,
    /// Current count of open (unreconciled) transactions — updated on each scrape.
    pub open_transactions_count: IntGauge,
    /// Current count of accounts — updated on each scrape.
    pub accounts_count: IntGauge,
    registry: Registry,
}

impl TreasuryMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

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

        Ok(Self {
            accounts_created_total,
            transactions_recorded_total,
            statements_imported_total,
            open_transactions_count,
            accounts_count,
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
    // Operational gauges will be refreshed from DB once schema is available (bd-1vrz).
    // For now, encode whatever counters have been incremented in-process.
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

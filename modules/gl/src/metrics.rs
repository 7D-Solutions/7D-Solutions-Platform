//! Prometheus metrics for GL module
//!
//! Phase 16: Operational observability for financial operations
//!
//! Metrics exposed:
//! - journal_entries_total: Total count of journal entries created
//! - posting_errors_total: Total count of posting errors encountered
//!
//! Design principle: Metrics must not mask errors or miscount entries.
//! All counters are append-only and never decrease.

use prometheus::{IntCounter, Encoder, TextEncoder, Registry};
use projections::metrics::ProjectionMetrics;
use std::sync::Arc;
use axum::{extract::State, http::StatusCode};

/// GL metrics registry
#[derive(Clone)]
pub struct GlMetrics {
    pub journal_entries_total: IntCounter,
    pub posting_errors_total: IntCounter,
    pub projection_metrics: ProjectionMetrics,
    registry: Registry,
}

impl GlMetrics {
    /// Create new GL metrics registry
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let journal_entries_total = IntCounter::new(
            "gl_journal_entries_total",
            "Total number of journal entries created"
        )?;

        let posting_errors_total = IntCounter::new(
            "gl_posting_errors_total",
            "Total number of posting errors encountered"
        )?;

        registry.register(Box::new(journal_entries_total.clone()))?;
        registry.register(Box::new(posting_errors_total.clone()))?;

        // Initialize projection metrics
        let projection_metrics = ProjectionMetrics::new()
            .map_err(|e| prometheus::Error::Msg(format!("Failed to create projection metrics: {}", e)))?;

        Ok(Self {
            journal_entries_total,
            posting_errors_total,
            projection_metrics,
            registry,
        })
    }

    /// Get registry for /metrics endpoint
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// Axum handler for /metrics endpoint
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();

    // Gather metrics from both GL metrics and projection metrics
    let mut metric_families = app_state.metrics.registry().gather();
    let projection_metric_families = app_state.metrics.projection_metrics.registry().gather();
    metric_families.extend(projection_metric_families);

    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer)
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e)
        ))?;

    String::from_utf8(buffer)
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to convert metrics to string: {}", e)
        ))
}

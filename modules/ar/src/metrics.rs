//! Prometheus metrics for AR module
//!
//! This module provides operational observability for the AR service,
//! exposing metrics about invoice lifecycle and system health.

use axum::{
    extract::State,
    http::StatusCode,
};
use prometheus::{Encoder, Histogram, IntCounter, Registry, TextEncoder};
use std::sync::Arc;

/// AR-specific metrics registry
pub struct ArMetrics {
    pub invoices_created_total: IntCounter,
    pub invoices_paid_total: IntCounter,
    pub invoice_age_seconds: Histogram,
    registry: Registry,
}

impl ArMetrics {
    /// Create a new metrics registry with AR-specific metrics
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Counter: Total invoices created
        let invoices_created_total = IntCounter::new(
            "ar_invoices_created_total",
            "Total number of invoices created",
        )?;
        registry.register(Box::new(invoices_created_total.clone()))?;

        // Counter: Total invoices paid
        let invoices_paid_total = IntCounter::new(
            "ar_invoices_paid_total",
            "Total number of invoices marked as paid",
        )?;
        registry.register(Box::new(invoices_paid_total.clone()))?;

        // Histogram: Invoice age in seconds (time from creation to payment)
        let invoice_age_seconds = Histogram::with_opts(
            prometheus::HistogramOpts::new(
                "ar_invoice_age_seconds",
                "Time in seconds from invoice creation to payment",
            )
            .buckets(vec![
                60.0,       // 1 minute
                300.0,      // 5 minutes
                900.0,      // 15 minutes
                3600.0,     // 1 hour
                86400.0,    // 1 day
                604800.0,   // 1 week
                2592000.0,  // 30 days
            ]),
        )?;
        registry.register(Box::new(invoice_age_seconds.clone()))?;

        Ok(Self {
            invoices_created_total,
            invoices_paid_total,
            invoice_age_seconds,
            registry,
        })
    }

    /// Get the underlying registry (for gathering metrics)
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// Axum handler for /metrics endpoint
///
/// Returns Prometheus-formatted metrics in text/plain format
pub async fn metrics_handler(
    State(app_state): State<Arc<crate::AppState>>,
) -> Result<String, (StatusCode, String)> {
    let encoder = TextEncoder::new();
    let metric_families = app_state.metrics.registry().gather();

    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| {
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

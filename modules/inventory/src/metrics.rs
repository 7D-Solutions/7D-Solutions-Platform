//! Prometheus metrics for Inventory module
//!
//! Metrics exposed:
//! - inventory_operations_total: Total count of inventory operations
//!
//! Design principle: Metrics must not mask errors or miscount operations.
//! All counters are append-only and never decrease.

use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use std::sync::Arc;

/// Inventory metrics registry
#[derive(Clone)]
pub struct InventoryMetrics {
    pub inventory_operations_total: IntCounter,
    registry: Registry,
}

impl InventoryMetrics {
    /// Create new Inventory metrics registry
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let inventory_operations_total = IntCounter::new(
            "inventory_operations_total",
            "Total number of inventory operations processed",
        )?;

        registry.register(Box::new(inventory_operations_total.clone()))?;

        Ok(Self {
            inventory_operations_total,
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
    let metric_families = app_state.metrics.registry().gather();

    let mut buffer = vec![];
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
            format!("Failed to convert metrics to string: {}", e),
        )
    })
}

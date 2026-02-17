//! Prometheus metrics for Subscriptions module
//!
//! This module provides operational observability for the Subscriptions service,
//! exposing metrics about billing cycles and subscription lifecycle.

use axum::{
    extract::State,
    http::StatusCode,
};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use projections::metrics::ProjectionMetrics;
use std::sync::Arc;

/// Subscriptions-specific metrics registry
pub struct SubscriptionsMetrics {
    pub cycles_attempted_total: IntCounter,
    pub cycles_completed_total: IntCounter,
    pub subscription_churn_total: IntCounter,
    pub projection_metrics: ProjectionMetrics,
    registry: Registry,
}

impl SubscriptionsMetrics {
    /// Create a new metrics registry with Subscriptions-specific metrics
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Counter: Total billing cycles attempted
        let cycles_attempted_total = IntCounter::new(
            "subscriptions_cycles_attempted_total",
            "Total number of billing cycles attempted",
        )?;
        registry.register(Box::new(cycles_attempted_total.clone()))?;

        // Counter: Total billing cycles completed successfully
        let cycles_completed_total = IntCounter::new(
            "subscriptions_cycles_completed_total",
            "Total number of billing cycles completed successfully",
        )?;
        registry.register(Box::new(cycles_completed_total.clone()))?;

        // Counter: Total subscription cancellations (churn)
        let subscription_churn_total = IntCounter::new(
            "subscriptions_churn_total",
            "Total number of subscription cancellations",
        )?;
        registry.register(Box::new(subscription_churn_total.clone()))?;

        // Initialize projection metrics
        let projection_metrics = ProjectionMetrics::new()?;

        Ok(Self {
            cycles_attempted_total,
            cycles_completed_total,
            subscription_churn_total,
            projection_metrics,
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

    // Gather metrics from both Subscriptions metrics and projection metrics
    let mut metric_families = app_state.metrics.registry().gather();
    let projection_metric_families = app_state.metrics.projection_metrics.registry().gather();
    metric_families.extend(projection_metric_families);

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

//! Projection metrics
//!
//! Provides operational observability for projection freshness and health:
//! - projection_lag_ms: Time difference between now and last processed event
//! - projection_last_applied_age_seconds: Time since cursor was last updated
//! - projection_backlog_count: Number of unprocessed events (module-specific)
//!
//! # Usage
//!
//! ```rust,no_run
//! use projections::metrics::ProjectionMetrics;
//! use projections::cursor::ProjectionCursor;
//!
//! async fn update_metrics(
//!     metrics: &ProjectionMetrics,
//!     cursor: &ProjectionCursor,
//! ) {
//!     metrics.record_cursor_state(cursor);
//! }
//! ```

use crate::cursor::ProjectionCursor;
use chrono::Utc;
use prometheus::{Gauge, GaugeVec, Opts, Registry};

/// Projection health and freshness metrics
pub struct ProjectionMetrics {
    /// Time difference in milliseconds between now and the last processed event
    /// Labels: projection_name, tenant_id
    pub projection_lag_ms: GaugeVec,

    /// Time in seconds since the projection cursor was last updated
    /// Labels: projection_name, tenant_id
    pub projection_last_applied_age_seconds: GaugeVec,

    /// Number of unprocessed events in the backlog (optional, module-specific)
    /// Labels: projection_name, tenant_id
    pub projection_backlog_count: GaugeVec,

    registry: Registry,
}

impl ProjectionMetrics {
    /// Create a new projection metrics instance
    ///
    /// # Errors
    ///
    /// Returns an error if metric registration fails.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Gauge: Projection lag in milliseconds
        let projection_lag_ms = GaugeVec::new(
            Opts::new(
                "projection_lag_ms",
                "Time in milliseconds between now and the last processed event",
            ),
            &["projection_name", "tenant_id"],
        )?;
        registry.register(Box::new(projection_lag_ms.clone()))?;

        // Gauge: Time since last cursor update
        let projection_last_applied_age_seconds = GaugeVec::new(
            Opts::new(
                "projection_last_applied_age_seconds",
                "Time in seconds since the projection cursor was last updated",
            ),
            &["projection_name", "tenant_id"],
        )?;
        registry.register(Box::new(projection_last_applied_age_seconds.clone()))?;

        // Gauge: Backlog count
        let projection_backlog_count = GaugeVec::new(
            Opts::new(
                "projection_backlog_count",
                "Number of unprocessed events in the projection backlog",
            ),
            &["projection_name", "tenant_id"],
        )?;
        registry.register(Box::new(projection_backlog_count.clone()))?;

        Ok(Self {
            projection_lag_ms,
            projection_last_applied_age_seconds,
            projection_backlog_count,
            registry,
        })
    }

    /// Record projection metrics from a cursor
    ///
    /// Updates lag and last_applied_age metrics based on cursor timestamps.
    /// Does not update backlog_count - use `record_backlog` for that.
    pub fn record_cursor_state(&self, cursor: &ProjectionCursor) {
        let now = Utc::now();

        // Calculate lag: how old is the last processed event?
        let lag_ms = (now - cursor.last_event_occurred_at)
            .num_milliseconds()
            .max(0) as f64;

        // Calculate last applied age: how long since we last updated the cursor?
        let last_applied_age_seconds = (now - cursor.updated_at)
            .num_seconds()
            .max(0) as f64;

        // Update metrics
        self.projection_lag_ms
            .with_label_values(&[&cursor.projection_name, &cursor.tenant_id])
            .set(lag_ms);

        self.projection_last_applied_age_seconds
            .with_label_values(&[&cursor.projection_name, &cursor.tenant_id])
            .set(last_applied_age_seconds);
    }

    /// Record backlog count for a projection
    ///
    /// This is module-specific and should be called with the result of
    /// counting unprocessed events in the source stream.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use projections::metrics::ProjectionMetrics;
    /// # async fn example(metrics: &ProjectionMetrics) {
    /// // Module-specific logic to count unprocessed events
    /// let backlog: i64 = 42; // From database query
    /// metrics.record_backlog("invoice_summary", "tenant-123", backlog);
    /// # }
    /// ```
    pub fn record_backlog(&self, projection_name: &str, tenant_id: &str, count: i64) {
        self.projection_backlog_count
            .with_label_values(&[projection_name, tenant_id])
            .set(count as f64);
    }

    /// Get the underlying registry for gathering metrics
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

impl Default for ProjectionMetrics {
    fn default() -> Self {
        Self::new().expect("Failed to create projection metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_record_cursor_state() {
        let metrics = ProjectionMetrics::new().unwrap();

        let cursor = ProjectionCursor {
            projection_name: "test_projection".to_string(),
            tenant_id: "tenant-123".to_string(),
            last_event_id: uuid::Uuid::new_v4(),
            last_event_occurred_at: Utc::now() - Duration::seconds(30),
            updated_at: Utc::now() - Duration::seconds(5),
            events_processed: 100,
        };

        metrics.record_cursor_state(&cursor);

        // Verify metrics were recorded (basic smoke test)
        let metric_families = metrics.registry.gather();
        assert!(metric_families.len() >= 2); // lag and last_applied_age at minimum
    }

    #[test]
    fn test_record_backlog() {
        let metrics = ProjectionMetrics::new().unwrap();

        metrics.record_backlog("test_projection", "tenant-123", 42);

        // Verify backlog was recorded
        let metric_families = metrics.registry.gather();
        assert!(!metric_families.is_empty());
    }

    #[test]
    fn test_metrics_labels() {
        let metrics = ProjectionMetrics::new().unwrap();

        // Test with multiple projections
        let cursor1 = ProjectionCursor {
            projection_name: "projection_a".to_string(),
            tenant_id: "tenant-1".to_string(),
            last_event_id: uuid::Uuid::new_v4(),
            last_event_occurred_at: Utc::now() - Duration::seconds(10),
            updated_at: Utc::now() - Duration::seconds(2),
            events_processed: 50,
        };

        let cursor2 = ProjectionCursor {
            projection_name: "projection_b".to_string(),
            tenant_id: "tenant-2".to_string(),
            last_event_id: uuid::Uuid::new_v4(),
            last_event_occurred_at: Utc::now() - Duration::seconds(60),
            updated_at: Utc::now() - Duration::seconds(10),
            events_processed: 200,
        };

        metrics.record_cursor_state(&cursor1);
        metrics.record_cursor_state(&cursor2);

        // Both projections should have their own metrics
        let metric_families = metrics.registry.gather();
        assert!(metric_families.len() >= 2);
    }
}

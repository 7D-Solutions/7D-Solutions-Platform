//! Fallback policy configuration and execution.
//!
//! Defines when and how to use HTTP fallback for stale projections.

use super::circuit_breaker::{CircuitBreaker, FallbackMetrics};
use crate::cursor::ProjectionCursor;
use chrono::Utc;
use std::future::Future;
use std::time::Duration;
use tokio::time::timeout;

/// Result type for fallback operations
pub type FallbackResult<T> = Result<T, FallbackError>;

/// Errors that can occur during fallback operations
#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    #[error("Fallback budget exceeded: {budget_ms}ms")]
    BudgetExceeded { budget_ms: u64 },

    #[error("Circuit breaker open: {failures} consecutive failures")]
    CircuitOpen { failures: u32 },

    #[error("Fallback execution failed: {0}")]
    ExecutionFailed(String),
}

/// Fallback policy configuration
///
/// Defines when and how to use HTTP fallback for stale projections.
#[derive(Debug, Clone)]
pub struct FallbackPolicy {
    /// Maximum allowed projection lag before fallback activates (milliseconds)
    pub staleness_threshold_ms: i64,

    /// Maximum time allowed for fallback call (milliseconds)
    pub budget_ms: u64,
}

impl FallbackPolicy {
    /// Create a new fallback policy
    ///
    /// # Arguments
    ///
    /// * `staleness_threshold_ms` - Max projection lag before fallback (e.g., 5000ms = 5s)
    /// * `budget_ms` - Max time for fallback call (e.g., 200ms)
    pub fn new(staleness_threshold_ms: i64, budget_ms: u64) -> Self {
        Self {
            staleness_threshold_ms,
            budget_ms,
        }
    }

    /// Check if a projection is stale beyond threshold
    ///
    /// Returns true if the projection lag exceeds the configured threshold.
    pub fn is_stale(&self, cursor: &ProjectionCursor) -> bool {
        let now = Utc::now();
        let lag_ms = (now - cursor.last_event_occurred_at).num_milliseconds();
        lag_ms > self.staleness_threshold_ms
    }

    /// Execute fallback with time budget enforcement
    ///
    /// Wraps the fallback function with timeout and metrics recording.
    /// Updates circuit breaker state based on success/failure.
    ///
    /// # Type Parameters
    ///
    /// * `T` - Return type of the fallback function
    /// * `F` - Async function that performs the fallback (e.g., HTTP call)
    ///
    /// # Arguments
    ///
    /// * `metrics` - Metrics for recording invocation and latency
    /// * `circuit` - Circuit breaker to track failures
    /// * `projection_name` - Name of the projection for metric labels
    /// * `tenant_id` - Tenant ID for metric labels
    /// * `fallback_fn` - Async function that performs the HTTP fallback
    ///
    /// # Returns
    ///
    /// * `Ok(T)` - Fallback succeeded within budget
    /// * `Err(FallbackError::BudgetExceeded)` - Fallback exceeded time budget
    /// * `Err(FallbackError::CircuitOpen)` - Circuit breaker is open
    /// * `Err(FallbackError::ExecutionFailed)` - Fallback function returned error
    pub async fn execute_with_budget<T, F>(
        &self,
        metrics: &FallbackMetrics,
        circuit: &CircuitBreaker,
        projection_name: &str,
        tenant_id: &str,
        fallback_fn: F,
    ) -> FallbackResult<T>
    where
        F: Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>>,
    {
        // Check circuit breaker state
        if !circuit.is_closed() {
            let failures = circuit.failure_count();
            return Err(FallbackError::CircuitOpen { failures });
        }

        // Record fallback invocation
        metrics.record_invocation(projection_name, tenant_id);

        // Execute with time budget
        let start = std::time::Instant::now();
        let budget = Duration::from_millis(self.budget_ms);

        let result = timeout(budget, fallback_fn).await;

        let elapsed_ms = start.elapsed().as_millis() as f64;

        match result {
            Ok(Ok(value)) => {
                // Success: record latency and reset circuit
                metrics.record_latency(projection_name, tenant_id, elapsed_ms);
                circuit.record_success();
                Ok(value)
            }
            Ok(Err(e)) => {
                // Execution failed: record failure and update circuit
                metrics.record_latency(projection_name, tenant_id, elapsed_ms);
                circuit.record_failure();
                Err(FallbackError::ExecutionFailed(e.to_string()))
            }
            Err(_) => {
                // Timeout: record failure and update circuit
                metrics.record_latency(projection_name, tenant_id, elapsed_ms);
                circuit.record_failure();
                Err(FallbackError::BudgetExceeded {
                    budget_ms: self.budget_ms,
                })
            }
        }
    }
}

impl Default for FallbackPolicy {
    /// Default policy: 5s staleness threshold, 200ms budget
    fn default() -> Self {
        Self::new(5000, 200)
    }
}

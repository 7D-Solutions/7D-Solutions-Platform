//! HTTP fallback policy for projection staleness
//!
//! Provides circuit breaker and time budget enforcement when projections are stale/unavailable.
//! This prevents cascading failures and thundering herd scenarios when projections fall behind.
//!
//! # Design Principles
//!
//! 1. **Staleness threshold**: Only activate fallback when projection exceeds configured lag
//! 2. **Time budget**: Fallback calls must complete within budget or fail fast
//! 3. **Circuit breaker**: Trip after sustained failures to prevent repeated attempts
//! 4. **Metrics**: Track invocation count and latency for observability
//!
//! # Example
//!
//! ```rust,no_run
//! use projections::fallback::{FallbackPolicy, FallbackMetrics, CircuitBreaker};
//! use projections::cursor::ProjectionCursor;
//! use std::time::Duration;
//!
//! async fn query_with_fallback(
//!     policy: &FallbackPolicy,
//!     metrics: &FallbackMetrics,
//!     circuit: &CircuitBreaker,
//!     cursor: Option<&ProjectionCursor>,
//! ) -> Result<String, Box<dyn std::error::Error>> {
//!     // Check if projection is stale
//!     if let Some(cursor) = cursor {
//!         if policy.is_stale(cursor) {
//!             // Try HTTP fallback if circuit is closed
//!             if circuit.is_closed() {
//!                 return policy.execute_with_budget(
//!                     metrics,
//!                     circuit,
//!                     || Box::pin(async {
//!                         // Call write service HTTP API
//!                         Ok("fallback data".to_string())
//!                     })
//!                 ).await;
//!             }
//!         }
//!     }
//!
//!     // Query projection normally
//!     Ok("projection data".to_string())
//! }
//! ```

use crate::cursor::ProjectionCursor;
use chrono::Utc;
use prometheus::{CounterVec, HistogramVec, Opts, Registry};
use std::future::Future;
use std::sync::{Arc, Mutex};
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

/// Fallback metrics
///
/// Tracks invocation count and latency for HTTP fallback operations.
#[derive(Clone)]
pub struct FallbackMetrics {
    /// Counter for fallback invocations
    /// Labels: projection_name, tenant_id
    fallback_invocation_count: CounterVec,

    /// Histogram for fallback latency (milliseconds)
    /// Labels: projection_name, tenant_id
    fallback_latency_ms: HistogramVec,

    registry: Registry,
}

impl FallbackMetrics {
    /// Create new fallback metrics
    ///
    /// # Errors
    ///
    /// Returns error if metric registration fails.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Counter: Fallback invocation count
        let fallback_invocation_count = CounterVec::new(
            Opts::new(
                "projection_fallback_invocation_count",
                "Number of times HTTP fallback was invoked for stale projections",
            ),
            &["projection_name", "tenant_id"],
        )?;
        registry.register(Box::new(fallback_invocation_count.clone()))?;

        // Histogram: Fallback latency
        let fallback_latency_ms = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "projection_fallback_latency_ms",
                "Latency of HTTP fallback calls in milliseconds",
            )
            .buckets(vec![10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0]),
            &["projection_name", "tenant_id"],
        )?;
        registry.register(Box::new(fallback_latency_ms.clone()))?;

        Ok(Self {
            fallback_invocation_count,
            fallback_latency_ms,
            registry,
        })
    }

    /// Record a fallback invocation
    pub fn record_invocation(&self, projection_name: &str, tenant_id: &str) {
        self.fallback_invocation_count
            .with_label_values(&[projection_name, tenant_id])
            .inc();
    }

    /// Record fallback latency
    pub fn record_latency(&self, projection_name: &str, tenant_id: &str, latency_ms: f64) {
        self.fallback_latency_ms
            .with_label_values(&[projection_name, tenant_id])
            .observe(latency_ms);
    }

    /// Get the underlying registry for gathering metrics
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

impl Default for FallbackMetrics {
    fn default() -> Self {
        Self::new().expect("Failed to create fallback metrics")
    }
}

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    /// Circuit is closed - fallback is allowed
    Closed,
    /// Circuit is open - fallback is blocked
    Open,
}

/// Circuit breaker for fallback protection
///
/// Tracks consecutive failures and opens the circuit after threshold is exceeded.
/// This prevents cascading failures and thundering herd scenarios.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    state: Arc<Mutex<CircuitBreakerState>>,
}

#[derive(Debug)]
struct CircuitBreakerState {
    /// Current circuit state
    state: CircuitState,
    /// Count of consecutive failures
    consecutive_failures: u32,
    /// Threshold before circuit opens
    failure_threshold: u32,
    /// Number of consecutive successes needed to close circuit
    success_threshold: u32,
    /// Count of consecutive successes (when recovering)
    consecutive_successes: u32,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    ///
    /// # Arguments
    ///
    /// * `failure_threshold` - Number of consecutive failures before opening (e.g., 5)
    /// * `success_threshold` - Number of consecutive successes to close circuit (e.g., 2)
    pub fn new(failure_threshold: u32, success_threshold: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(CircuitBreakerState {
                state: CircuitState::Closed,
                consecutive_failures: 0,
                failure_threshold,
                success_threshold,
                consecutive_successes: 0,
            })),
        }
    }

    /// Check if circuit is closed (fallback allowed)
    pub fn is_closed(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.state == CircuitState::Closed
    }

    /// Get current failure count
    pub fn failure_count(&self) -> u32 {
        let state = self.state.lock().unwrap();
        state.consecutive_failures
    }

    /// Record a successful fallback
    ///
    /// Resets failure counter. If circuit is open, increments success counter
    /// and may close the circuit if threshold is met.
    pub fn record_success(&self) {
        let mut state = self.state.lock().unwrap();

        match state.state {
            CircuitState::Closed => {
                // Reset failure counter
                state.consecutive_failures = 0;
                state.consecutive_successes = 0;
            }
            CircuitState::Open => {
                // Increment success counter
                state.consecutive_successes += 1;

                // Check if we should close the circuit
                if state.consecutive_successes >= state.success_threshold {
                    state.state = CircuitState::Closed;
                    state.consecutive_failures = 0;
                    state.consecutive_successes = 0;
                }
            }
        }
    }

    /// Record a failed fallback
    ///
    /// Increments failure counter and may open the circuit if threshold is exceeded.
    pub fn record_failure(&self) {
        let mut state = self.state.lock().unwrap();

        // Reset success counter
        state.consecutive_successes = 0;

        // Increment failure counter
        state.consecutive_failures += 1;

        // Check if we should open the circuit
        if state.consecutive_failures >= state.failure_threshold {
            state.state = CircuitState::Open;
        }
    }

    /// Reset circuit breaker to closed state
    ///
    /// Useful for testing or manual intervention.
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.state = CircuitState::Closed;
        state.consecutive_failures = 0;
        state.consecutive_successes = 0;
    }
}

impl Default for CircuitBreaker {
    /// Default: 5 failures to open, 2 successes to close
    fn default() -> Self {
        Self::new(5, 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    #[test]
    fn test_policy_is_stale() {
        let policy = FallbackPolicy::new(5000, 200);

        // Fresh cursor (1 second lag)
        let fresh_cursor = ProjectionCursor {
            projection_name: "test".to_string(),
            tenant_id: "tenant-1".to_string(),
            last_event_id: uuid::Uuid::new_v4(),
            last_event_occurred_at: Utc::now() - ChronoDuration::seconds(1),
            updated_at: Utc::now(),
            events_processed: 10,
        };
        assert!(!policy.is_stale(&fresh_cursor));

        // Stale cursor (10 second lag)
        let stale_cursor = ProjectionCursor {
            projection_name: "test".to_string(),
            tenant_id: "tenant-1".to_string(),
            last_event_id: uuid::Uuid::new_v4(),
            last_event_occurred_at: Utc::now() - ChronoDuration::seconds(10),
            updated_at: Utc::now(),
            events_processed: 10,
        };
        assert!(policy.is_stale(&stale_cursor));
    }

    #[test]
    fn test_circuit_breaker_open_close() {
        let breaker = CircuitBreaker::new(3, 2);

        // Initially closed
        assert!(breaker.is_closed());
        assert_eq!(breaker.failure_count(), 0);

        // Record failures
        breaker.record_failure();
        assert!(breaker.is_closed());
        assert_eq!(breaker.failure_count(), 1);

        breaker.record_failure();
        assert!(breaker.is_closed());
        assert_eq!(breaker.failure_count(), 2);

        breaker.record_failure();
        assert!(!breaker.is_closed()); // Circuit opens
        assert_eq!(breaker.failure_count(), 3);

        // Record success - should not close yet
        breaker.record_success();
        assert!(!breaker.is_closed());

        // Second success - circuit closes
        breaker.record_success();
        assert!(breaker.is_closed());
        assert_eq!(breaker.failure_count(), 0);
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let breaker = CircuitBreaker::new(2, 2);

        breaker.record_failure();
        breaker.record_failure();
        assert!(!breaker.is_closed());

        breaker.reset();
        assert!(breaker.is_closed());
        assert_eq!(breaker.failure_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_with_budget_success() {
        let policy = FallbackPolicy::new(5000, 500);
        let metrics = FallbackMetrics::new().unwrap();
        let circuit = CircuitBreaker::new(5, 2);

        let result = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                "test_projection",
                "tenant-1",
                async { Ok::<_, Box<dyn std::error::Error + Send + Sync>>(42) },
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert!(circuit.is_closed());
    }

    #[tokio::test]
    async fn test_execute_with_budget_timeout() {
        let policy = FallbackPolicy::new(5000, 50); // 50ms budget
        let metrics = FallbackMetrics::new().unwrap();
        let circuit = CircuitBreaker::new(5, 2);

        let result = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                "test_projection",
                "tenant-1",
                async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(42)
                },
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FallbackError::BudgetExceeded { .. }
        ));
        assert_eq!(circuit.failure_count(), 1);
    }

    #[tokio::test]
    async fn test_execute_with_circuit_open() {
        let policy = FallbackPolicy::new(5000, 200);
        let metrics = FallbackMetrics::new().unwrap();
        let circuit = CircuitBreaker::new(2, 2);

        // Trip the circuit
        circuit.record_failure();
        circuit.record_failure();
        assert!(!circuit.is_closed());

        // Attempt fallback - should fail immediately
        let result = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                "test_projection",
                "tenant-1",
                async { Ok::<_, Box<dyn std::error::Error + Send + Sync>>(42) },
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FallbackError::CircuitOpen { failures: 2 }
        ));
    }
}

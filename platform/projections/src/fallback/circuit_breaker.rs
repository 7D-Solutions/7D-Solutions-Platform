//! Circuit breaker and metrics for fallback protection.
//!
//! Tracks consecutive failures and opens the circuit after threshold is exceeded.
//! This prevents cascading failures and thundering herd scenarios.

use prometheus::{CounterVec, HistogramVec, Opts, Registry};
use std::sync::{Arc, Mutex};

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
    /// Create new fallback metrics with an isolated registry.
    ///
    /// Metrics are registered into a private `Registry`. Use
    /// [`FallbackMetrics::new_with_registry`] to wire into the global
    /// Prometheus registry so the module's `/metrics` endpoint sees them.
    ///
    /// # Errors
    ///
    /// Returns error if metric registration fails.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_registry(&Registry::new())
    }

    /// Create new fallback metrics registered into `registry`.
    ///
    /// Pass `prometheus::default_registry()` to make these metrics visible
    /// to `prometheus::gather()` (i.e., the module's `/metrics` endpoint).
    ///
    /// # Errors
    ///
    /// Returns error if metric registration fails (e.g., duplicate names in
    /// the same registry).
    pub fn new_with_registry(registry: &Registry) -> Result<Self, Box<dyn std::error::Error>> {
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
            registry: registry.clone(),
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
        let state = self.state.lock().expect("circuit breaker mutex poisoned");
        state.state == CircuitState::Closed
    }

    /// Get current failure count
    pub fn failure_count(&self) -> u32 {
        let state = self.state.lock().expect("circuit breaker mutex poisoned");
        state.consecutive_failures
    }

    /// Record a successful fallback
    ///
    /// Resets failure counter. If circuit is open, increments success counter
    /// and may close the circuit if threshold is met.
    pub fn record_success(&self) {
        let mut state = self.state.lock().expect("circuit breaker mutex poisoned");

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
        let mut state = self.state.lock().expect("circuit breaker mutex poisoned");

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
        let mut state = self.state.lock().expect("circuit breaker mutex poisoned");
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

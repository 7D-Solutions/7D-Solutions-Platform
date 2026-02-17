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
//!                     "my_projection",
//!                     "tenant-1",
//!                     async {
//!                         // Call write service HTTP API
//!                         Ok("fallback data".to_string())
//!                     }
//!                 ).await.map_err(|e| e.into());
//!             }
//!         }
//!     }
//!
//!     // Query projection normally
//!     Ok("projection data".to_string())
//! }
//! ```

mod circuit_breaker;
mod policy;

#[cfg(test)]
mod tests;

pub use circuit_breaker::{CircuitBreaker, FallbackMetrics};
pub use policy::{FallbackError, FallbackPolicy, FallbackResult};

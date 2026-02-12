//! Consumer retry logic with exponential backoff
//!
//! Provides retry functionality for event consumers to handle transient failures
//! before events are sent to the Dead Letter Queue (DLQ).

use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial backoff duration (doubles on each retry)
    pub initial_backoff: Duration,
    /// Maximum backoff duration to cap exponential growth
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Retry a fallible async operation with exponential backoff
///
/// # Arguments
/// * `operation` - The async operation to retry (must be Send)
/// * `config` - Retry configuration
/// * `context` - Context string for logging (e.g., "process_payment_event")
///
/// # Returns
/// * `Ok(T)` if operation succeeds within max_attempts
/// * `Err(E)` if all retries are exhausted
///
/// # Example
/// ```rust
/// use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
///
/// # async fn example() -> Result<(), String> {
/// let config = RetryConfig::default();
/// let result = retry_with_backoff(
///     || async { Ok::<_, String>(42) },
///     &config,
///     "example_operation"
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn retry_with_backoff<F, Fut, T, E>(
    operation: F,
    config: &RetryConfig,
    context: &str,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display + Send,
{
    let mut attempt = 0;
    let mut backoff = config.initial_backoff;

    loop {
        attempt += 1;

        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    debug!(
                        context = %context,
                        attempt = attempt,
                        "Operation succeeded after retry"
                    );
                }
                return Ok(result);
            }
            Err(e) => {
                if attempt >= config.max_attempts {
                    warn!(
                        context = %context,
                        attempts = attempt,
                        error = %e,
                        "Operation failed after max retries"
                    );
                    return Err(e);
                }

                warn!(
                    context = %context,
                    attempt = attempt,
                    max_attempts = config.max_attempts,
                    backoff_ms = backoff.as_millis(),
                    error = %e,
                    "Operation failed, retrying with backoff"
                );

                sleep(backoff).await;

                // Exponential backoff with cap
                backoff = std::cmp::min(backoff * 2, config.max_backoff);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn test_retry_succeeds_first_attempt() {
        let config = RetryConfig::default();
        let result = retry_with_backoff(
            || async { Ok::<_, String>(42) },
            &config,
            "test_operation",
        )
        .await;

        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let config = RetryConfig::default();
        let attempts = Arc::new(Mutex::new(0));
        let attempts_clone = attempts.clone();

        let result = retry_with_backoff(
            || {
                let attempts = attempts_clone.clone();
                async move {
                    let mut count = attempts.lock().unwrap();
                    *count += 1;
                    if *count < 3 {
                        Err(format!("Attempt {}", *count))
                    } else {
                        Ok(42)
                    }
                }
            },
            &config,
            "test_operation",
        )
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempts.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn test_retry_fails_after_max_attempts() {
        let config = RetryConfig {
            max_attempts: 2,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(20),
        };

        let result = retry_with_backoff(
            || async { Err::<i32, _>("persistent error") },
            &config,
            "test_operation",
        )
        .await;

        assert_eq!(result, Err("persistent error"));
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let config = RetryConfig {
            max_attempts: 4,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
        };

        let start = std::time::Instant::now();
        let attempts = Arc::new(Mutex::new(0));
        let attempts_clone = attempts.clone();

        let _result = retry_with_backoff(
            || {
                let attempts = attempts_clone.clone();
                async move {
                    let mut count = attempts.lock().unwrap();
                    *count += 1;
                    Err::<i32, _>("error")
                }
            },
            &config,
            "test_operation",
        )
        .await;

        let elapsed = start.elapsed();

        // Should have waited: 10ms + 20ms + 40ms = 70ms minimum
        // But capped at 50ms for last retry: 10ms + 20ms + 50ms = 80ms
        assert!(elapsed >= Duration::from_millis(70));
        assert_eq!(*attempts.lock().unwrap(), 4);
    }
}

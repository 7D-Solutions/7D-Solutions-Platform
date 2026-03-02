use super::*;
use crate::cursor::ProjectionCursor;
use chrono::{Duration as ChronoDuration, Utc};
use std::time::Duration;

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
        .execute_with_budget(&metrics, &circuit, "test_projection", "tenant-1", async {
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(42)
        })
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
        .execute_with_budget(&metrics, &circuit, "test_projection", "tenant-1", async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(42)
        })
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
        .execute_with_budget(&metrics, &circuit, "test_projection", "tenant-1", async {
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(42)
        })
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        FallbackError::CircuitOpen { failures: 2 }
    ));
}

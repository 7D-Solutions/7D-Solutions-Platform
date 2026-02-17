//! E2E tests for projection fallback with circuit breaker (bd-17a3)
//!
//! Tests the HTTP fallback policy when projections are stale/unavailable:
//! 1. Fallback only activates when projection exceeds staleness threshold
//! 2. Circuit breaker trips under sustained failures
//! 3. Metrics are emitted for fallback count and latency

use chrono::{Duration, Utc};
use projections::{
    cursor::ProjectionCursor, CircuitBreaker, FallbackError, FallbackMetrics, FallbackPolicy,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Helper to initialize test database with projection cursors schema
async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/ar_test".to_string());

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Ensure projection_cursors table exists
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS projection_cursors (
            projection_name TEXT NOT NULL,
            tenant_id TEXT NOT NULL,
            last_event_id UUID NOT NULL,
            last_event_occurred_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            events_processed BIGINT NOT NULL DEFAULT 0,
            PRIMARY KEY (projection_name, tenant_id)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("Failed to create projection_cursors table");

    pool
}

/// Helper to clean up test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM projection_cursors WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to clean up test data");
}

/// Test 1: Fallback only activates when projection is stale beyond threshold
#[tokio::test]
async fn test_fallback_staleness_threshold() {
    let pool = setup_test_db().await;
    let tenant_id = "test-tenant-staleness";
    let projection_name = "test_projection";

    // Create fresh cursor (1 second lag - within threshold)
    let fresh_event_id = Uuid::new_v4();
    let fresh_occurred_at = Utc::now() - Duration::seconds(1);

    ProjectionCursor::save(
        &pool,
        projection_name,
        tenant_id,
        fresh_event_id,
        fresh_occurred_at,
    )
    .await
    .expect("Failed to save fresh cursor");

    // Load cursor and check staleness
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    let policy = FallbackPolicy::new(5000, 200); // 5s threshold
    assert!(
        !policy.is_stale(&cursor),
        "Fresh cursor should not be stale"
    );

    // Create stale cursor (10 second lag - beyond threshold)
    let stale_event_id = Uuid::new_v4();
    let stale_occurred_at = Utc::now() - Duration::seconds(10);

    ProjectionCursor::save(
        &pool,
        projection_name,
        tenant_id,
        stale_event_id,
        stale_occurred_at,
    )
    .await
    .expect("Failed to save stale cursor");

    let stale_cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    assert!(
        policy.is_stale(&stale_cursor),
        "Stale cursor should be detected"
    );

    cleanup_test_data(&pool, tenant_id).await;
}

/// Test 2: Circuit breaker trips under sustained failures
#[tokio::test]
async fn test_circuit_breaker_trips() {
    let policy = FallbackPolicy::new(5000, 200);
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(3, 2); // 3 failures to open

    let projection_name = "test_projection";
    let tenant_id = "test-tenant-circuit";

    // Circuit should start closed
    assert!(circuit.is_closed(), "Circuit should start closed");

    // Simulate 3 consecutive failures
    for i in 0..3 {
        let result = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                projection_name,
                tenant_id,
                async {
                    // Simulate failure
                    Err::<(), _>(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Simulated failure",
                    ))
                        as Box<dyn std::error::Error + Send + Sync>)
                },
            )
            .await;

        assert!(result.is_err(), "Fallback should fail");

        if i < 2 {
            assert!(circuit.is_closed(), "Circuit should still be closed");
        } else {
            assert!(!circuit.is_closed(), "Circuit should be open after 3 failures");
        }
    }

    // Circuit is now open - next attempt should fail immediately without calling fallback
    let result = policy
        .execute_with_budget(
            &metrics,
            &circuit,
            projection_name,
            tenant_id,
            async {
                // This should not execute
                Ok::<i32, _>(42)
            },
        )
        .await;

    assert!(result.is_err(), "Should fail due to open circuit");
    assert!(
        matches!(result.unwrap_err(), FallbackError::CircuitOpen { .. }),
        "Error should be CircuitOpen"
    );
}

/// Test 3: Circuit breaker closes after successful recoveries
#[tokio::test]
async fn test_circuit_breaker_recovery() {
    let policy = FallbackPolicy::new(5000, 200);
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(3, 2); // 3 failures to open, 2 successes to close

    let projection_name = "test_projection";
    let tenant_id = "test-tenant-recovery";

    // Trip the circuit
    for _ in 0..3 {
        let _ = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                projection_name,
                tenant_id,
                async {
                    Err::<(), _>(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Failure",
                    ))
                        as Box<dyn std::error::Error + Send + Sync>)
                },
            )
            .await;
    }

    assert!(!circuit.is_closed(), "Circuit should be open");

    // Record 2 successful operations (manually for testing)
    circuit.record_success();
    assert!(!circuit.is_closed(), "Circuit should still be open after 1 success");

    circuit.record_success();
    assert!(circuit.is_closed(), "Circuit should close after 2 successes");
}

/// Test 4: Time budget enforcement
#[tokio::test]
async fn test_time_budget_enforcement() {
    let policy = FallbackPolicy::new(5000, 100); // 100ms budget
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(5, 2);

    let projection_name = "test_projection";
    let tenant_id = "test-tenant-budget";

    // Fast operation - should succeed
    let result = policy
        .execute_with_budget(
            &metrics,
            &circuit,
            projection_name,
            tenant_id,
            async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Ok::<i32, _>(42)
            },
        )
        .await;

    assert!(result.is_ok(), "Fast operation should succeed");
    assert_eq!(result.unwrap(), 42);

    // Slow operation - should timeout
    let result = policy
        .execute_with_budget(
            &metrics,
            &circuit,
            projection_name,
            tenant_id,
            async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                Ok::<i32, _>(42)
            },
        )
        .await;

    assert!(result.is_err(), "Slow operation should timeout");
    assert!(
        matches!(result.unwrap_err(), FallbackError::BudgetExceeded { .. }),
        "Error should be BudgetExceeded"
    );
}

/// Test 5: Metrics are recorded for fallback invocations
#[tokio::test]
async fn test_fallback_metrics() {
    let policy = FallbackPolicy::new(5000, 200);
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(5, 2);

    let projection_name = "test_projection";
    let tenant_id = "test-tenant-metrics";

    // Successful fallback
    let result = policy
        .execute_with_budget(
            &metrics,
            &circuit,
            projection_name,
            tenant_id,
            async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok::<i32, _>(42)
            },
        )
        .await;

    assert!(result.is_ok(), "Fallback should succeed");

    // Gather metrics
    let metric_families = metrics.registry().gather();
    assert!(
        !metric_families.is_empty(),
        "Metrics should be recorded"
    );

    // Verify counter and histogram metrics exist
    let has_counter = metric_families
        .iter()
        .any(|mf| mf.get_name() == "projection_fallback_invocation_count");
    let has_histogram = metric_families
        .iter()
        .any(|mf| mf.get_name() == "projection_fallback_latency_ms");

    assert!(has_counter, "Counter metric should exist");
    assert!(has_histogram, "Histogram metric should exist");
}

/// Test 6: End-to-end fallback flow with projection cursor
#[tokio::test]
async fn test_e2e_fallback_flow() {
    let pool = setup_test_db().await;
    let tenant_id = "test-tenant-e2e";
    let projection_name = "invoice_projection";

    // Create stale projection cursor
    let stale_event_id = Uuid::new_v4();
    let stale_occurred_at = Utc::now() - Duration::seconds(10);

    ProjectionCursor::save(
        &pool,
        projection_name,
        tenant_id,
        stale_event_id,
        stale_occurred_at,
    )
    .await
    .expect("Failed to save stale cursor");

    // Load cursor
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    // Check staleness
    let policy = FallbackPolicy::new(5000, 200);
    assert!(policy.is_stale(&cursor), "Cursor should be stale");

    // Execute fallback
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(5, 2);

    let result = policy
        .execute_with_budget(
            &metrics,
            &circuit,
            projection_name,
            tenant_id,
            async {
                // Simulate HTTP fallback call
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok::<String, _>("fallback_data".to_string())
            },
        )
        .await;

    assert!(result.is_ok(), "Fallback should succeed");
    assert_eq!(result.unwrap(), "fallback_data");

    cleanup_test_data(&pool, tenant_id).await;
}

/// Test 7: Circuit breaker prevents repeated failures
#[tokio::test]
async fn test_circuit_prevents_cascading_failures() {
    let policy = FallbackPolicy::new(5000, 200);
    let metrics = FallbackMetrics::default();
    let circuit = CircuitBreaker::new(3, 2);

    let projection_name = "test_projection";
    let tenant_id = "test-tenant-cascading";

    let failure_count = Arc::new(std::sync::atomic::AtomicU32::new(0));

    // Trip the circuit with 3 failures
    for _ in 0..3 {
        let count = failure_count.clone();
        let _ = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                projection_name,
                tenant_id,
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err::<(), _>(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Failure",
                    ))
                        as Box<dyn std::error::Error + Send + Sync>)
                },
            )
            .await;
    }

    assert!(!circuit.is_closed(), "Circuit should be open");
    let failures_before = failure_count.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(failures_before, 3, "Should have 3 failed attempts");

    // Next 5 attempts should fail immediately without executing fallback
    for _ in 0..5 {
        let count = failure_count.clone();
        let result = policy
            .execute_with_budget(
                &metrics,
                &circuit,
                projection_name,
                tenant_id,
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok::<(), _>(())
                },
            )
            .await;

        assert!(
            matches!(result, Err(FallbackError::CircuitOpen { .. })),
            "Should fail with CircuitOpen"
        );
    }

    let failures_after = failure_count.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        failures_after, 3,
        "Circuit breaker should prevent additional fallback attempts"
    );
}

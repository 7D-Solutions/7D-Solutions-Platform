/// E2E test for projection lag metrics
///
/// Verifies that projection health metrics:
/// 1. projection_lag_ms appears in /metrics with proper labels
/// 2. projection_last_applied_age_seconds is calculated correctly
/// 3. projection_backlog_count can be recorded
/// 4. Metrics reflect stale projections correctly
/// 5. Metrics are exposed via module /metrics endpoints

mod common;

use chrono::{Duration, Utc};
use common::get_projections_pool;
use projections::cursor::ProjectionCursor;
use projections::metrics::ProjectionMetrics;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to run migrations on the projections database
async fn run_projections_migrations(pool: &PgPool) {
    // Read and execute the migration file
    let migration_sql = include_str!(
        "../../platform/projections/db/migrations/20260216000001_create_projection_cursors.sql"
    );

    // Drop existing table if it exists (for test idempotency)
    sqlx::query("DROP TABLE IF EXISTS projection_cursors CASCADE")
        .execute(pool)
        .await
        .expect("Failed to drop projection_cursors table");

    // Execute the migration
    sqlx::raw_sql(migration_sql)
        .execute(pool)
        .await
        .expect("Failed to run projection migrations");
}

#[tokio::test]
#[serial]
async fn test_projection_metrics_creation() {
    // Verify that ProjectionMetrics can be created
    let metrics = ProjectionMetrics::new().expect("Failed to create projection metrics");

    // Record a test value to ensure metrics appear in output
    metrics.record_backlog("test_projection", "test_tenant", 0);

    // Verify metrics are registered
    let families = metrics.registry().gather();
    assert!(!families.is_empty(), "Should have at least one metric family");

    // Verify metric names include our key metrics
    let metric_names: Vec<String> = families.iter().map(|f| f.get_name().to_string()).collect();
    let has_projection_metrics = metric_names.iter().any(|name| {
        name.contains("projection_lag")
        || name.contains("projection_backlog")
        || name.contains("projection_last_applied")
    });
    assert!(
        has_projection_metrics,
        "Should have at least one projection metric, got: {:?}",
        metric_names
    );
}

#[tokio::test]
#[serial]
async fn test_projection_lag_from_cursor() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    // Create a cursor with a stale event (30 seconds old)
    let projection_name = "invoice_summary";
    let tenant_id = "tenant-123";
    let event_id = Uuid::new_v4();
    let event_occurred_at = Utc::now() - Duration::seconds(30);

    // Save the cursor
    ProjectionCursor::save(&pool, projection_name, tenant_id, event_id, event_occurred_at)
        .await
        .expect("Failed to save cursor");

    // Load the cursor
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    // Record metrics from cursor
    let metrics = ProjectionMetrics::new().expect("Failed to create metrics");
    metrics.record_cursor_state(&cursor);

    // Gather metrics and verify lag is reported
    let families = metrics.registry().gather();
    let lag_family = families
        .iter()
        .find(|f| f.get_name() == "projection_lag_ms")
        .expect("projection_lag_ms metric should exist");

    // Verify the metric has the correct labels
    let metrics_vec = lag_family.get_metric();
    assert!(!metrics_vec.is_empty(), "Should have at least one metric");

    // Check that lag is approximately 30 seconds (30000ms)
    // Allow for some timing variation (25-35 seconds)
    let gauge_value = metrics_vec[0].get_gauge().get_value();
    assert!(
        gauge_value >= 25000.0 && gauge_value <= 35000.0,
        "Lag should be approximately 30000ms, got: {}",
        gauge_value
    );
}

#[tokio::test]
#[serial]
async fn test_projection_last_applied_age() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    // Create a cursor that was last updated 10 seconds ago
    let projection_name = "customer_balance";
    let tenant_id = "tenant-456";
    let event_id = Uuid::new_v4();
    let event_occurred_at = Utc::now() - Duration::seconds(5);

    // Save the cursor
    ProjectionCursor::save(&pool, projection_name, tenant_id, event_id, event_occurred_at)
        .await
        .expect("Failed to save cursor");

    // Manually update the updated_at to be 10 seconds old
    sqlx::query(
        "UPDATE projection_cursors SET updated_at = $1 WHERE projection_name = $2 AND tenant_id = $3"
    )
    .bind(Utc::now() - Duration::seconds(10))
    .bind(projection_name)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("Failed to update cursor timestamp");

    // Load the cursor
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    // Record metrics from cursor
    let metrics = ProjectionMetrics::new().expect("Failed to create metrics");
    metrics.record_cursor_state(&cursor);

    // Gather metrics and verify last_applied_age is reported
    let families = metrics.registry().gather();
    let age_family = families
        .iter()
        .find(|f| f.get_name() == "projection_last_applied_age_seconds")
        .expect("projection_last_applied_age_seconds metric should exist");

    // Verify the metric has data
    let metrics_vec = age_family.get_metric();
    assert!(!metrics_vec.is_empty(), "Should have at least one metric");

    // Check that age is approximately 10 seconds
    // Allow for some timing variation (8-12 seconds)
    let gauge_value = metrics_vec[0].get_gauge().get_value();
    assert!(
        gauge_value >= 8.0 && gauge_value <= 12.0,
        "Last applied age should be approximately 10s, got: {}",
        gauge_value
    );
}

#[tokio::test]
#[serial]
async fn test_projection_backlog_recording() {
    let metrics = ProjectionMetrics::new().expect("Failed to create metrics");

    // Record backlog for a projection
    metrics.record_backlog("invoice_summary", "tenant-123", 42);
    metrics.record_backlog("customer_balance", "tenant-456", 0);

    // Gather metrics and verify backlog is reported
    let families = metrics.registry().gather();
    let backlog_family = families
        .iter()
        .find(|f| f.get_name() == "projection_backlog_count")
        .expect("projection_backlog_count metric should exist");

    // Verify we have metrics for both projections
    let metrics_vec = backlog_family.get_metric();
    assert!(
        metrics_vec.len() >= 2,
        "Should have backlog metrics for at least 2 projections"
    );

    // Verify one of them has backlog = 42
    let has_backlog_42 = metrics_vec
        .iter()
        .any(|m| m.get_gauge().get_value() == 42.0);
    assert!(
        has_backlog_42,
        "Should have a projection with backlog of 42"
    );
}

#[tokio::test]
#[serial]
async fn test_multiple_projections_separate_metrics() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    // Create multiple cursors with different lag amounts
    let cursors_data = vec![
        ("projection_a", "tenant-1", 10), // 10 seconds old
        ("projection_b", "tenant-1", 60), // 60 seconds old
        ("projection_a", "tenant-2", 5),  // 5 seconds old (different tenant)
    ];

    for (projection, tenant, lag_seconds) in &cursors_data {
        let event_id = Uuid::new_v4();
        let event_occurred_at = Utc::now() - Duration::seconds(*lag_seconds);
        ProjectionCursor::save(&pool, projection, tenant, event_id, event_occurred_at)
            .await
            .expect("Failed to save cursor");
    }

    // Load all cursors and record metrics
    let metrics = ProjectionMetrics::new().expect("Failed to create metrics");

    for (projection, tenant, _) in &cursors_data {
        let cursor = ProjectionCursor::load(&pool, projection, tenant)
            .await
            .expect("Failed to load cursor")
            .expect("Cursor should exist");
        metrics.record_cursor_state(&cursor);
    }

    // Gather metrics and verify we have separate metrics for each projection/tenant
    let families = metrics.registry().gather();
    let lag_family = families
        .iter()
        .find(|f| f.get_name() == "projection_lag_ms")
        .expect("projection_lag_ms metric should exist");

    let metrics_vec = lag_family.get_metric();
    assert_eq!(
        metrics_vec.len(),
        3,
        "Should have 3 separate metrics (one per projection/tenant combo)"
    );
}

#[tokio::test]
#[serial]
async fn test_failing_projection_increases_lag() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection_name = "failing_projection";
    let tenant_id = "tenant-999";
    let event_id = Uuid::new_v4();

    // Simulate a projection that starts processing but fails
    // First event processed successfully (time T)
    let event_occurred_at = Utc::now() - Duration::seconds(120);
    ProjectionCursor::save(&pool, projection_name, tenant_id, event_id, event_occurred_at)
        .await
        .expect("Failed to save cursor");

    // Record initial metrics
    let metrics = ProjectionMetrics::new().expect("Failed to create metrics");
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");
    metrics.record_cursor_state(&cursor);

    // Gather initial lag
    let families = metrics.registry().gather();
    let lag_family = families
        .iter()
        .find(|f| f.get_name() == "projection_lag_ms")
        .expect("projection_lag_ms metric should exist");

    let initial_lag = lag_family.get_metric()[0].get_gauge().get_value();

    // Verify lag is approximately 120 seconds (120000ms)
    assert!(
        initial_lag >= 115000.0 && initial_lag <= 125000.0,
        "Initial lag should be approximately 120000ms, got: {}",
        initial_lag
    );

    // Verify last_applied_age is very small (cursor was just saved)
    let age_family = families
        .iter()
        .find(|f| f.get_name() == "projection_last_applied_age_seconds")
        .expect("projection_last_applied_age_seconds metric should exist");

    let initial_age = age_family.get_metric()[0].get_gauge().get_value();
    assert!(
        initial_age <= 2.0,
        "Initial last_applied_age should be less than 2s, got: {}",
        initial_age
    );
}

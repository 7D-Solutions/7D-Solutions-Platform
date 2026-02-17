/// E2E test for projection cursor contract
///
/// Verifies that the projection cursor tracking system:
/// 1. Persists cursor position per (projection_name, tenant_id)
/// 2. Enforces idempotent apply semantics (no duplicate event processing)
/// 3. Updates cursors transactionally with read-model writes
/// 4. Returns correct apply status (true = applied, false = already processed)
/// 5. Supports deterministic rebuild capability

mod common;

use chrono::Utc;
use common::get_projections_pool;
use projections::cursor::{try_apply_event, CursorError, ProjectionCursor};
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

/// Helper to create a test read-model table
async fn create_test_read_model(pool: &PgPool) {
    // Drop if exists
    sqlx::query("DROP TABLE IF EXISTS test_customer_balances CASCADE")
        .execute(pool)
        .await
        .expect("Failed to drop test table");

    // Create a simple read-model table for testing
    sqlx::query(
        r#"
        CREATE TABLE test_customer_balances (
            customer_id VARCHAR(100) PRIMARY KEY,
            tenant_id VARCHAR(100) NOT NULL,
            balance BIGINT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("Failed to create test_customer_balances table");
}

#[tokio::test]
#[serial]
async fn test_cursor_load_nonexistent() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    // Load cursor for projection that doesn't exist yet
    let cursor = ProjectionCursor::load(&pool, "customer_balance", "tenant-123")
        .await
        .expect("Failed to load cursor");

    // Should return None for nonexistent cursor
    assert!(cursor.is_none(), "Cursor should not exist for new projection");
}

#[tokio::test]
#[serial]
async fn test_cursor_save_and_load() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-456";
    let event_id = Uuid::new_v4();
    let event_occurred_at = Utc::now();

    // Save a cursor
    ProjectionCursor::save(&pool, projection_name, tenant_id, event_id, event_occurred_at)
        .await
        .expect("Failed to save cursor");

    // Load it back
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    assert_eq!(cursor.projection_name, projection_name);
    assert_eq!(cursor.tenant_id, tenant_id);
    assert_eq!(cursor.last_event_id, event_id);
    assert_eq!(cursor.events_processed, 1);
}

#[tokio::test]
#[serial]
async fn test_cursor_save_updates_existing() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-789";

    // Save first event
    let event1_id = Uuid::new_v4();
    ProjectionCursor::save(&pool, projection_name, tenant_id, event1_id, Utc::now())
        .await
        .expect("Failed to save first cursor");

    // Save second event (should update, not insert)
    let event2_id = Uuid::new_v4();
    ProjectionCursor::save(&pool, projection_name, tenant_id, event2_id, Utc::now())
        .await
        .expect("Failed to save second cursor");

    // Load and verify
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    // Should have the second event ID and events_processed = 2
    assert_eq!(cursor.last_event_id, event2_id);
    assert_eq!(cursor.events_processed, 2);
}

#[tokio::test]
#[serial]
async fn test_is_processed_returns_false_for_new_event() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let event_id = Uuid::new_v4();

    // Check if event is processed (no cursor exists yet)
    let is_processed =
        ProjectionCursor::is_processed(&pool, "new_projection", "tenant-new", event_id)
            .await
            .expect("Failed to check is_processed");

    assert!(!is_processed, "New event should not be marked as processed");
}

#[tokio::test]
#[serial]
async fn test_is_processed_returns_true_for_same_event() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-same";
    let event_id = Uuid::new_v4();

    // Save cursor with this event
    ProjectionCursor::save(&pool, projection_name, tenant_id, event_id, Utc::now())
        .await
        .expect("Failed to save cursor");

    // Check if the same event is processed
    let is_processed = ProjectionCursor::is_processed(&pool, projection_name, tenant_id, event_id)
        .await
        .expect("Failed to check is_processed");

    assert!(is_processed, "Same event should be marked as processed");
}

#[tokio::test]
#[serial]
async fn test_is_processed_returns_false_for_different_event() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-diff";
    let event1_id = Uuid::new_v4();
    let event2_id = Uuid::new_v4();

    // Save cursor with first event
    ProjectionCursor::save(&pool, projection_name, tenant_id, event1_id, Utc::now())
        .await
        .expect("Failed to save cursor");

    // Check if a different event is processed
    let is_processed =
        ProjectionCursor::is_processed(&pool, projection_name, tenant_id, event2_id)
            .await
            .expect("Failed to check is_processed");

    assert!(!is_processed, "Different event should not be marked as processed");
}

#[tokio::test]
#[serial]
async fn test_try_apply_event_applies_new_event() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;
    create_test_read_model(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-apply-new";
    let customer_id = "cust-123";
    let event_id = Uuid::new_v4();

    // Insert initial customer balance
    sqlx::query("INSERT INTO test_customer_balances (customer_id, tenant_id, balance) VALUES ($1, $2, $3)")
        .bind(customer_id)
        .bind(tenant_id)
        .bind(0_i64)
        .execute(&pool)
        .await
        .expect("Failed to insert customer");

    // Start a transaction
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    // Try to apply a new event
    let applied = try_apply_event(
        &mut tx,
        projection_name,
        tenant_id,
        event_id,
        Utc::now(),
        |tx| Box::pin(async move {
            // Update customer balance
            sqlx::query("UPDATE test_customer_balances SET balance = balance + $1 WHERE customer_id = $2 AND tenant_id = $3")
                .bind(100_i64)
                .bind(customer_id)
                .bind(tenant_id)
                .execute(tx)
                .await
                .map_err(CursorError::from)?;
            Ok(())
        }),
    )
    .await
    .expect("Failed to apply event");

    // Commit transaction
    tx.commit().await.expect("Failed to commit transaction");

    // Verify event was applied
    assert!(applied, "Event should have been applied");

    // Verify cursor was saved
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    assert_eq!(cursor.last_event_id, event_id);
    assert_eq!(cursor.events_processed, 1);

    // Verify read-model was updated
    let balance: i64 = sqlx::query_scalar(
        "SELECT balance FROM test_customer_balances WHERE customer_id = $1 AND tenant_id = $2",
    )
    .bind(customer_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch balance");

    assert_eq!(balance, 100);
}

#[tokio::test]
#[serial]
async fn test_try_apply_event_skips_duplicate() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;
    create_test_read_model(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-apply-dup";
    let customer_id = "cust-456";
    let event_id = Uuid::new_v4();

    // Insert initial customer balance
    sqlx::query("INSERT INTO test_customer_balances (customer_id, tenant_id, balance) VALUES ($1, $2, $3)")
        .bind(customer_id)
        .bind(tenant_id)
        .bind(0_i64)
        .execute(&pool)
        .await
        .expect("Failed to insert customer");

    // First apply
    let mut tx1 = pool.begin().await.expect("Failed to begin transaction");
    let applied1 = try_apply_event(
        &mut tx1,
        projection_name,
        tenant_id,
        event_id,
        Utc::now(),
        |tx| Box::pin(async move {
            sqlx::query("UPDATE test_customer_balances SET balance = balance + $1 WHERE customer_id = $2 AND tenant_id = $3")
                .bind(100_i64)
                .bind(customer_id)
                .bind(tenant_id)
                .execute(tx)
                .await
                .map_err(CursorError::from)?;
            Ok(())
        }),
    )
    .await
    .expect("Failed to apply event first time");
    tx1.commit().await.expect("Failed to commit transaction");

    assert!(applied1, "First apply should succeed");

    // Second apply with same event_id (should be idempotent)
    let mut tx2 = pool.begin().await.expect("Failed to begin transaction");
    let applied2 = try_apply_event(
        &mut tx2,
        projection_name,
        tenant_id,
        event_id,
        Utc::now(),
        |tx| Box::pin(async move {
            sqlx::query("UPDATE test_customer_balances SET balance = balance + $1 WHERE customer_id = $2 AND tenant_id = $3")
                .bind(100_i64)
                .bind(customer_id)
                .bind(tenant_id)
                .execute(tx)
                .await
                .map_err(CursorError::from)?;
            Ok(())
        }),
    )
    .await
    .expect("Failed to apply event second time");
    tx2.commit().await.expect("Failed to commit transaction");

    assert!(!applied2, "Second apply should be skipped (idempotent)");

    // Verify cursor still has the same event
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor")
        .expect("Cursor should exist");

    assert_eq!(cursor.last_event_id, event_id);
    assert_eq!(cursor.events_processed, 1); // Should still be 1, not 2

    // Verify balance was only updated once
    let balance: i64 = sqlx::query_scalar(
        "SELECT balance FROM test_customer_balances WHERE customer_id = $1 AND tenant_id = $2",
    )
    .bind(customer_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch balance");

    assert_eq!(balance, 100); // Should be 100, not 200
}

#[tokio::test]
#[serial]
async fn test_try_apply_event_transactional_rollback() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;
    create_test_read_model(&pool).await;

    let projection_name = "customer_balance";
    let tenant_id = "tenant-apply-rollback";
    let customer_id = "cust-789";
    let event_id = Uuid::new_v4();

    // Insert initial customer balance
    sqlx::query("INSERT INTO test_customer_balances (customer_id, tenant_id, balance) VALUES ($1, $2, $3)")
        .bind(customer_id)
        .bind(tenant_id)
        .bind(0_i64)
        .execute(&pool)
        .await
        .expect("Failed to insert customer");

    // Start a transaction
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    // Try to apply event that will fail during apply_fn
    let result = try_apply_event(
        &mut tx,
        projection_name,
        tenant_id,
        event_id,
        Utc::now(),
        |tx| Box::pin(async move {
            // Update customer balance
            sqlx::query("UPDATE test_customer_balances SET balance = balance + $1 WHERE customer_id = $2 AND tenant_id = $3")
                .bind(100_i64)
                .bind(customer_id)
                .bind(tenant_id)
                .execute(tx)
                .await
                .map_err(CursorError::from)?;

            // Simulate an error during processing
            Err(CursorError::Database(sqlx::Error::RowNotFound))
        }),
    )
    .await;

    // Should return error
    assert!(result.is_err(), "Should fail due to simulated error");

    // Rollback transaction
    tx.rollback().await.expect("Failed to rollback transaction");

    // Verify cursor was NOT saved (transactional rollback)
    let cursor = ProjectionCursor::load(&pool, projection_name, tenant_id)
        .await
        .expect("Failed to load cursor");

    assert!(cursor.is_none(), "Cursor should not exist after rollback");

    // Verify balance was NOT updated (transactional rollback)
    let balance: i64 = sqlx::query_scalar(
        "SELECT balance FROM test_customer_balances WHERE customer_id = $1 AND tenant_id = $2",
    )
    .bind(customer_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch balance");

    assert_eq!(balance, 0); // Should still be 0
}

#[tokio::test]
#[serial]
async fn test_multi_tenant_isolation() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection_name = "customer_balance";
    let tenant1_id = "tenant-1";
    let tenant2_id = "tenant-2";
    let event1_id = Uuid::new_v4();
    let event2_id = Uuid::new_v4();

    // Save cursor for tenant 1
    ProjectionCursor::save(&pool, projection_name, tenant1_id, event1_id, Utc::now())
        .await
        .expect("Failed to save cursor for tenant 1");

    // Save cursor for tenant 2
    ProjectionCursor::save(&pool, projection_name, tenant2_id, event2_id, Utc::now())
        .await
        .expect("Failed to save cursor for tenant 2");

    // Load cursor for tenant 1
    let cursor1 = ProjectionCursor::load(&pool, projection_name, tenant1_id)
        .await
        .expect("Failed to load cursor for tenant 1")
        .expect("Cursor should exist for tenant 1");

    // Load cursor for tenant 2
    let cursor2 = ProjectionCursor::load(&pool, projection_name, tenant2_id)
        .await
        .expect("Failed to load cursor for tenant 2")
        .expect("Cursor should exist for tenant 2");

    // Verify isolation - each tenant has their own cursor
    assert_eq!(cursor1.tenant_id, tenant1_id);
    assert_eq!(cursor1.last_event_id, event1_id);
    assert_eq!(cursor2.tenant_id, tenant2_id);
    assert_eq!(cursor2.last_event_id, event2_id);
    assert_ne!(cursor1.last_event_id, cursor2.last_event_id);
}

#[tokio::test]
#[serial]
async fn test_multi_projection_isolation() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    let projection1_name = "customer_balance";
    let projection2_name = "invoice_summary";
    let tenant_id = "tenant-multi-proj";
    let event1_id = Uuid::new_v4();
    let event2_id = Uuid::new_v4();

    // Save cursor for projection 1
    ProjectionCursor::save(&pool, projection1_name, tenant_id, event1_id, Utc::now())
        .await
        .expect("Failed to save cursor for projection 1");

    // Save cursor for projection 2
    ProjectionCursor::save(&pool, projection2_name, tenant_id, event2_id, Utc::now())
        .await
        .expect("Failed to save cursor for projection 2");

    // Load cursor for projection 1
    let cursor1 = ProjectionCursor::load(&pool, projection1_name, tenant_id)
        .await
        .expect("Failed to load cursor for projection 1")
        .expect("Cursor should exist for projection 1");

    // Load cursor for projection 2
    let cursor2 = ProjectionCursor::load(&pool, projection2_name, tenant_id)
        .await
        .expect("Failed to load cursor for projection 2")
        .expect("Cursor should exist for projection 2");

    // Verify isolation - each projection has its own cursor
    assert_eq!(cursor1.projection_name, projection1_name);
    assert_eq!(cursor1.last_event_id, event1_id);
    assert_eq!(cursor2.projection_name, projection2_name);
    assert_eq!(cursor2.last_event_id, event2_id);
    assert_ne!(cursor1.last_event_id, cursor2.last_event_id);
}

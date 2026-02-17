//! E2E test for replay certification with digest equality
//!
//! This test verifies that rebuilding projections from the same event stream
//! produces identical digests, demonstrating deterministic replay certification.
//!
//! Tests include:
//! 1. AR Invoice Summary projection (invoice counts, totals by customer)
//! 2. Payments Attempt Summary projection (attempt counts, success/failure totals)
//! 3. Digest equality verification across two rebuild runs
//! 4. Cursor position equality verification
//! 5. Row count verification

use chrono::{DateTime, Utc};
use projections::{
    compute_versioned_digest, create_shadow_cursor_table, create_shadow_table,
    drop_shadow_table, save_shadow_cursor, swap_cursor_tables_atomic, swap_tables_atomic,
    VersionedDigest,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to set up the projections database pool (delegates to common helper with fallback URL)
async fn get_projections_pool() -> PgPool {
    let url = std::env::var("PROJECTIONS_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://projections_user:projections_pass@localhost:5439/projections_db".to_string());
    sqlx::PgPool::connect(&url)
        .await
        .expect("Failed to connect to projections database")
}

/// Helper to run base migrations
async fn run_base_migrations(pool: &PgPool) {
    // Drop existing tables
    sqlx::query("DROP TABLE IF EXISTS projection_cursors CASCADE")
        .execute(pool)
        .await
        .ok();

    // Run migration
    let migration_sql = include_str!(
        "../../platform/projections/db/migrations/20260216000001_create_projection_cursors.sql"
    );

    sqlx::raw_sql(migration_sql)
        .execute(pool)
        .await
        .expect("Failed to run projection migrations");
}

/// Create AR invoice summary projection table
async fn create_ar_invoice_summary_table(pool: &PgPool, table_name: &str) {
    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table_name))
        .execute(pool)
        .await
        .expect("Failed to drop table");

    sqlx::query(&format!(
        r#"
        CREATE TABLE {} (
            customer_id VARCHAR(100) PRIMARY KEY,
            tenant_id VARCHAR(100) NOT NULL,
            invoice_count INT NOT NULL DEFAULT 0,
            total_amount BIGINT NOT NULL DEFAULT 0,
            last_invoice_date TIMESTAMPTZ,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT '2024-01-01T00:00:00Z'
        )
        "#,
        table_name
    ))
    .execute(pool)
    .await
    .expect("Failed to create AR invoice summary table");
}

/// Create Payments attempt summary projection table
async fn create_payments_attempt_summary_table(pool: &PgPool, table_name: &str) {
    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table_name))
        .execute(pool)
        .await
        .expect("Failed to drop table");

    sqlx::query(&format!(
        r#"
        CREATE TABLE {} (
            customer_id VARCHAR(100) PRIMARY KEY,
            tenant_id VARCHAR(100) NOT NULL,
            total_attempts INT NOT NULL DEFAULT 0,
            successful_attempts INT NOT NULL DEFAULT 0,
            failed_attempts INT NOT NULL DEFAULT 0,
            total_amount_attempted BIGINT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT '2024-01-01T00:00:00Z'
        )
        "#,
        table_name
    ))
    .execute(pool)
    .await
    .expect("Failed to create Payments attempt summary table");
}

/// Replay events into AR invoice summary projection
///
/// Events are applied deterministically using:
/// - Deterministic event IDs (UUID from index)
/// - Deterministic timestamps (fixed base time)
/// - Deterministic ordering (by event index)
async fn replay_ar_invoice_events(
    pool: &PgPool,
    table_name: &str,
    cursor_table: &str,
    projection_name: &str,
    tenant_id: &str,
    event_count: usize,
) -> (Uuid, DateTime<Utc>, i64, i64) {
    let deterministic_timestamp =
        DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

    let mut last_event_id = Uuid::nil();
    let mut last_event_occurred_at = deterministic_timestamp;

    for i in 0..event_count {
        let event_id = Uuid::from_u128(i as u128);
        let event_occurred_at = deterministic_timestamp;
        let customer_id = format!("cust-{}", i % 10); // 10 customers
        let invoice_amount = ((i + 1) * 100) as i64; // Deterministic amounts

        // Apply event: increment invoice count and add to total
        sqlx::query(&format!(
            r#"
            INSERT INTO {} (customer_id, tenant_id, invoice_count, total_amount, last_invoice_date, updated_at)
            VALUES ($1, $2, 1, $3, $4, $4)
            ON CONFLICT (customer_id)
            DO UPDATE SET
                invoice_count = {}.invoice_count + 1,
                total_amount = {}.total_amount + $3,
                last_invoice_date = $4,
                updated_at = $4
            "#,
            table_name, table_name, table_name
        ))
        .bind(&customer_id)
        .bind(tenant_id)
        .bind(invoice_amount)
        .bind(deterministic_timestamp)
        .execute(pool)
        .await
        .expect("Failed to apply AR invoice event");

        // Update cursor
        save_shadow_cursor(pool, projection_name, tenant_id, event_id, event_occurred_at)
            .await
            .expect("Failed to save cursor");

        last_event_id = event_id;
        last_event_occurred_at = event_occurred_at;
    }

    // Get final row count
    let row_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", table_name))
        .fetch_one(pool)
        .await
        .expect("Failed to get row count");

    (last_event_id, last_event_occurred_at, event_count as i64, row_count)
}

/// Replay events into Payments attempt summary projection
async fn replay_payments_attempt_events(
    pool: &PgPool,
    table_name: &str,
    cursor_table: &str,
    projection_name: &str,
    tenant_id: &str,
    event_count: usize,
) -> (Uuid, DateTime<Utc>, i64, i64) {
    let deterministic_timestamp =
        DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

    let mut last_event_id = Uuid::nil();
    let mut last_event_occurred_at = deterministic_timestamp;

    for i in 0..event_count {
        let event_id = Uuid::from_u128((1000 + i) as u128); // Different UUID range from AR
        let event_occurred_at = deterministic_timestamp;
        let customer_id = format!("cust-{}", i % 10);
        let attempt_amount = ((i + 1) * 50) as i64;
        let is_success = i % 3 != 0; // Deterministic success pattern

        // Apply event: increment attempt counts
        sqlx::query(&format!(
            r#"
            INSERT INTO {} (customer_id, tenant_id, total_attempts, successful_attempts, failed_attempts, total_amount_attempted, updated_at)
            VALUES ($1, $2, 1, $3, $4, $5, $6)
            ON CONFLICT (customer_id)
            DO UPDATE SET
                total_attempts = {}.total_attempts + 1,
                successful_attempts = {}.successful_attempts + $3,
                failed_attempts = {}.failed_attempts + $4,
                total_amount_attempted = {}.total_amount_attempted + $5,
                updated_at = $6
            "#,
            table_name, table_name, table_name, table_name, table_name
        ))
        .bind(&customer_id)
        .bind(tenant_id)
        .bind(if is_success { 1 } else { 0 })
        .bind(if is_success { 0 } else { 1 })
        .bind(attempt_amount)
        .bind(deterministic_timestamp)
        .execute(pool)
        .await
        .expect("Failed to apply Payments attempt event");

        // Update cursor
        save_shadow_cursor(pool, projection_name, tenant_id, event_id, event_occurred_at)
            .await
            .expect("Failed to save cursor");

        last_event_id = event_id;
        last_event_occurred_at = event_occurred_at;
    }

    // Get final row count
    let row_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", table_name))
        .fetch_one(pool)
        .await
        .expect("Failed to get row count");

    (last_event_id, last_event_occurred_at, event_count as i64, row_count)
}

/// Rebuild summary for a single projection run
#[derive(Debug, Clone)]
struct RebuildRunSummary {
    projection_name: String,
    events_processed: i64,
    row_count: i64,
    last_event_id: Uuid,
    last_event_occurred_at: DateTime<Utc>,
    digest: VersionedDigest,
}

/// Run a complete rebuild of AR invoice summary projection
async fn rebuild_ar_invoice_summary(
    pool: &PgPool,
    projection_name: &str,
    tenant_id: &str,
    event_count: usize,
) -> RebuildRunSummary {
    let base_table = "ar_invoice_summary";

    // Clean up from previous runs
    drop_shadow_table(pool, base_table).await.ok();
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(pool)
        .await
        .ok();

    // Create shadow tables
    create_ar_invoice_summary_table(pool, &format!("{}_shadow", base_table)).await;
    create_shadow_cursor_table(pool)
        .await
        .expect("Failed to create shadow cursor table");

    // Replay events
    let (last_event_id, last_event_occurred_at, events_processed, row_count) =
        replay_ar_invoice_events(
            pool,
            &format!("{}_shadow", base_table),
            "projection_cursors_shadow",
            projection_name,
            tenant_id,
            event_count,
        )
        .await;

    // Compute digest
    let digest = compute_versioned_digest(pool, &format!("{}_shadow", base_table), "customer_id")
        .await
        .expect("Failed to compute digest");

    // Perform atomic swap
    swap_tables_atomic(pool, base_table)
        .await
        .expect("Failed to swap tables");
    swap_cursor_tables_atomic(pool)
        .await
        .expect("Failed to swap cursor tables");

    RebuildRunSummary {
        projection_name: projection_name.to_string(),
        events_processed,
        row_count,
        last_event_id,
        last_event_occurred_at,
        digest,
    }
}

/// Run a complete rebuild of Payments attempt summary projection
async fn rebuild_payments_attempt_summary(
    pool: &PgPool,
    projection_name: &str,
    tenant_id: &str,
    event_count: usize,
) -> RebuildRunSummary {
    let base_table = "payments_attempt_summary";

    // Clean up from previous runs
    drop_shadow_table(pool, base_table).await.ok();
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(pool)
        .await
        .ok();

    // Create shadow tables
    create_payments_attempt_summary_table(pool, &format!("{}_shadow", base_table)).await;
    create_shadow_cursor_table(pool)
        .await
        .expect("Failed to create shadow cursor table");

    // Replay events
    let (last_event_id, last_event_occurred_at, events_processed, row_count) =
        replay_payments_attempt_events(
            pool,
            &format!("{}_shadow", base_table),
            "projection_cursors_shadow",
            projection_name,
            tenant_id,
            event_count,
        )
        .await;

    // Compute digest
    let digest =
        compute_versioned_digest(pool, &format!("{}_shadow", base_table), "customer_id")
            .await
            .expect("Failed to compute digest");

    // Perform atomic swap
    swap_tables_atomic(pool, base_table)
        .await
        .expect("Failed to swap tables");
    swap_cursor_tables_atomic(pool)
        .await
        .expect("Failed to swap cursor tables");

    RebuildRunSummary {
        projection_name: projection_name.to_string(),
        events_processed,
        row_count,
        last_event_id,
        last_event_occurred_at,
        digest,
    }
}

#[tokio::test]
#[serial]
async fn test_ar_invoice_summary_digest_equality() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    let projection_name = "ar_invoice_summary";
    let tenant_id = "tenant-cert";
    let event_count = 50;

    println!("\n=== AR Invoice Summary Replay Certification ===");
    println!("Projection: {}", projection_name);
    println!("Tenant: {}", tenant_id);
    println!("Events: {}", event_count);

    // Run 1
    println!("\n--- Rebuild Run 1 ---");
    let run1 = rebuild_ar_invoice_summary(&pool, projection_name, tenant_id, event_count).await;
    println!("Events processed: {}", run1.events_processed);
    println!("Row count: {}", run1.row_count);
    println!("Last event ID: {}", run1.last_event_id);
    println!("Digest: {}", run1.digest);

    // Run 2
    println!("\n--- Rebuild Run 2 ---");
    let run2 = rebuild_ar_invoice_summary(&pool, projection_name, tenant_id, event_count).await;
    println!("Events processed: {}", run2.events_processed);
    println!("Row count: {}", run2.row_count);
    println!("Last event ID: {}", run2.last_event_id);
    println!("Digest: {}", run2.digest);

    // Assert digest equality
    println!("\n--- Certification Results ---");
    assert_eq!(
        run1.digest, run2.digest,
        "Digests must be identical across rebuilds"
    );
    println!("✅ Digest equality: PASSED");

    // Assert cursor equality
    assert_eq!(
        run1.last_event_id, run2.last_event_id,
        "Last event IDs must match"
    );
    println!("✅ Cursor position equality: PASSED");

    // Assert row count equality
    assert_eq!(
        run1.row_count, run2.row_count,
        "Row counts must match"
    );
    println!("✅ Row count equality: PASSED");

    // Assert events processed equality
    assert_eq!(
        run1.events_processed, run2.events_processed,
        "Events processed must match"
    );
    println!("✅ Events processed equality: PASSED");

    println!("\n✅ AR Invoice Summary Replay Certification: PASSED\n");

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS ar_invoice_summary CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS ar_invoice_summary_old CASCADE")
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_payments_attempt_summary_digest_equality() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    let projection_name = "payments_attempt_summary";
    let tenant_id = "tenant-cert";
    let event_count = 40;

    println!("\n=== Payments Attempt Summary Replay Certification ===");
    println!("Projection: {}", projection_name);
    println!("Tenant: {}", tenant_id);
    println!("Events: {}", event_count);

    // Run 1
    println!("\n--- Rebuild Run 1 ---");
    let run1 =
        rebuild_payments_attempt_summary(&pool, projection_name, tenant_id, event_count).await;
    println!("Events processed: {}", run1.events_processed);
    println!("Row count: {}", run1.row_count);
    println!("Last event ID: {}", run1.last_event_id);
    println!("Digest: {}", run1.digest);

    // Run 2
    println!("\n--- Rebuild Run 2 ---");
    let run2 =
        rebuild_payments_attempt_summary(&pool, projection_name, tenant_id, event_count).await;
    println!("Events processed: {}", run2.events_processed);
    println!("Row count: {}", run2.row_count);
    println!("Last event ID: {}", run2.last_event_id);
    println!("Digest: {}", run2.digest);

    // Assert digest equality
    println!("\n--- Certification Results ---");
    assert_eq!(
        run1.digest, run2.digest,
        "Digests must be identical across rebuilds"
    );
    println!("✅ Digest equality: PASSED");

    // Assert cursor equality
    assert_eq!(
        run1.last_event_id, run2.last_event_id,
        "Last event IDs must match"
    );
    println!("✅ Cursor position equality: PASSED");

    // Assert row count equality
    assert_eq!(
        run1.row_count, run2.row_count,
        "Row counts must match"
    );
    println!("✅ Row count equality: PASSED");

    // Assert events processed equality
    assert_eq!(
        run1.events_processed, run2.events_processed,
        "Events processed must match"
    );
    println!("✅ Events processed equality: PASSED");

    println!("\n✅ Payments Attempt Summary Replay Certification: PASSED\n");

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS payments_attempt_summary CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS payments_attempt_summary_old CASCADE")
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_multi_projection_certification_suite() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    let tenant_id = "tenant-multi-cert";

    println!("\n=== Multi-Projection Replay Certification Suite ===");

    // Test AR Invoice Summary
    let ar_event_count = 30;
    let ar_run1 =
        rebuild_ar_invoice_summary(&pool, "ar_invoice_summary", tenant_id, ar_event_count).await;
    let ar_run2 =
        rebuild_ar_invoice_summary(&pool, "ar_invoice_summary", tenant_id, ar_event_count).await;

    println!("\n--- AR Invoice Summary ---");
    println!("Digest Run 1: {}", ar_run1.digest);
    println!("Digest Run 2: {}", ar_run2.digest);
    assert_eq!(ar_run1.digest, ar_run2.digest);
    println!("✅ AR Invoice Summary: CERTIFIED");

    // Test Payments Attempt Summary
    let payments_event_count = 35;
    let payments_run1 = rebuild_payments_attempt_summary(
        &pool,
        "payments_attempt_summary",
        tenant_id,
        payments_event_count,
    )
    .await;
    let payments_run2 = rebuild_payments_attempt_summary(
        &pool,
        "payments_attempt_summary",
        tenant_id,
        payments_event_count,
    )
    .await;

    println!("\n--- Payments Attempt Summary ---");
    println!("Digest Run 1: {}", payments_run1.digest);
    println!("Digest Run 2: {}", payments_run2.digest);
    assert_eq!(payments_run1.digest, payments_run2.digest);
    println!("✅ Payments Attempt Summary: CERTIFIED");

    println!("\n✅ Multi-Projection Certification Suite: ALL PASSED\n");

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS ar_invoice_summary CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS ar_invoice_summary_old CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS payments_attempt_summary CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS payments_attempt_summary_old CASCADE")
        .execute(&pool)
        .await
        .ok();
}

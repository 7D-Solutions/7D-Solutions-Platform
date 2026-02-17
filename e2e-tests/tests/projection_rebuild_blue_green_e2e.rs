/// E2E test for projection rebuild with blue/green swap
///
/// Verifies that:
/// 1. Shadow tables can be created for rebuild isolation
/// 2. Events can be replayed into shadow tables
/// 3. Digest computation is deterministic
/// 4. Blue/green swap is atomic from reader perspective
/// 5. Two rebuilds with same event stream produce identical digests

mod common;

use chrono::Utc;
use common::get_projections_pool;
use projections::{
    compute_digest, create_shadow_cursor_table, create_shadow_table, drop_shadow_table,
    save_shadow_cursor, swap_cursor_tables_atomic, swap_tables_atomic,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to run base migrations (projection_cursors)
async fn run_base_migrations(pool: &PgPool) {
    // Drop existing indexes first to avoid conflicts
    sqlx::query("DROP INDEX IF EXISTS projection_cursors_updated_at CASCADE")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DROP INDEX IF EXISTS projection_cursors_tenant_id CASCADE")
        .execute(pool)
        .await
        .ok();

    // Drop table
    sqlx::query("DROP TABLE IF EXISTS projection_cursors CASCADE")
        .execute(pool)
        .await
        .expect("Failed to drop projection_cursors");

    // Run migration
    let migration_sql = include_str!(
        "../../platform/projections/db/migrations/20260216000001_create_projection_cursors.sql"
    );

    sqlx::raw_sql(migration_sql)
        .execute(pool)
        .await
        .expect("Failed to run projection migrations");
}

/// Helper to create a test projection table
async fn create_test_projection_table(pool: &PgPool, table_name: &str) {
    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table_name))
        .execute(pool)
        .await
        .expect("Failed to drop test table");

    sqlx::query(&format!(
        r#"
        CREATE TABLE {} (
            customer_id VARCHAR(100) PRIMARY KEY,
            tenant_id VARCHAR(100) NOT NULL,
            balance BIGINT NOT NULL DEFAULT 0,
            event_count INT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
        table_name
    ))
    .execute(pool)
    .await
    .expect("Failed to create test projection table");
}

/// Helper to simulate event replay into a table
async fn replay_events_into_table(
    pool: &PgPool,
    table_name: &str,
    cursor_table: &str,
    projection_name: &str,
    tenant_id: &str,
    event_count: usize,
) -> (Uuid, chrono::DateTime<Utc>, i64) {
    let mut last_event_id = Uuid::nil();
    let mut last_event_occurred_at = Utc::now();

    for i in 0..event_count {
        let event_id = Uuid::new_v4();
        let event_occurred_at = Utc::now();
        let customer_id = format!("cust-{}", i % 10); // 10 customers

        // Upsert customer balance (simulating event apply)
        sqlx::query(&format!(
            r#"
            INSERT INTO {} (customer_id, tenant_id, balance, event_count)
            VALUES ($1, $2, $3, 1)
            ON CONFLICT (customer_id)
            DO UPDATE SET
                balance = {}.balance + $3,
                event_count = {}.event_count + 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
            table_name, table_name, table_name
        ))
        .bind(&customer_id)
        .bind(tenant_id)
        .bind(100_i64) // Each event adds 100 to balance
        .execute(pool)
        .await
        .expect("Failed to apply event");

        // Update cursor (using provided cursor table name)
        if cursor_table == "projection_cursors_shadow" {
            save_shadow_cursor(pool, projection_name, tenant_id, event_id, event_occurred_at)
                .await
                .expect("Failed to save shadow cursor");
        } else {
            sqlx::query(&format!(
                r#"
                INSERT INTO {} (
                    projection_name,
                    tenant_id,
                    last_event_id,
                    last_event_occurred_at,
                    updated_at,
                    events_processed
                ) VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP, 1)
                ON CONFLICT (projection_name, tenant_id)
                DO UPDATE SET
                    last_event_id = EXCLUDED.last_event_id,
                    last_event_occurred_at = EXCLUDED.last_event_occurred_at,
                    updated_at = CURRENT_TIMESTAMP,
                    events_processed = {}.events_processed + 1
                "#,
                cursor_table, cursor_table
            ))
            .bind(projection_name)
            .bind(tenant_id)
            .bind(event_id)
            .bind(event_occurred_at)
            .execute(pool)
            .await
            .expect("Failed to save cursor");
        }

        last_event_id = event_id;
        last_event_occurred_at = event_occurred_at;
    }

    (last_event_id, last_event_occurred_at, event_count as i64)
}

#[tokio::test]
#[serial]
async fn test_create_shadow_table() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    // Clean up any existing shadow table
    drop_shadow_table(&pool, "test_balances")
        .await
        .ok();

    // Create shadow table
    let create_ddl = r#"
        CREATE TABLE test_balances_shadow (
            customer_id VARCHAR(100) PRIMARY KEY,
            tenant_id VARCHAR(100) NOT NULL,
            balance BIGINT NOT NULL DEFAULT 0
        )
    "#;

    create_shadow_table(&pool, "test_balances", create_ddl)
        .await
        .expect("Failed to create shadow table");

    // Verify shadow table exists
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = 'test_balances_shadow'
        )",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to check if shadow table exists");

    assert!(exists, "Shadow table should exist");

    // Clean up
    drop_shadow_table(&pool, "test_balances")
        .await
        .expect("Failed to drop shadow table");
}

#[tokio::test]
#[serial]
async fn test_shadow_cursor_table() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    // Drop existing shadow cursor table
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(&pool)
        .await
        .expect("Failed to drop shadow cursor table");

    // Create shadow cursor table
    create_shadow_cursor_table(&pool)
        .await
        .expect("Failed to create shadow cursor table");

    // Verify it exists
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.tables
            WHERE table_name = 'projection_cursors_shadow'
        )",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to check if shadow cursor table exists");

    assert!(exists, "Shadow cursor table should exist");

    // Test saving a cursor to shadow table
    let event_id = Uuid::new_v4();
    save_shadow_cursor(&pool, "test_projection", "tenant-123", event_id, Utc::now())
        .await
        .expect("Failed to save shadow cursor");

    // Verify cursor was saved
    let cursor: (Uuid,) = sqlx::query_as(
        "SELECT last_event_id FROM projection_cursors_shadow
         WHERE projection_name = $1 AND tenant_id = $2",
    )
    .bind("test_projection")
    .bind("tenant-123")
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch shadow cursor");

    assert_eq!(cursor.0, event_id);
}

#[tokio::test]
#[serial]
async fn test_compute_digest_deterministic() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    create_test_projection_table(&pool, "test_balances").await;

    // Insert test data in a specific order
    for i in 0..5 {
        let customer_id = format!("cust-{}", i);
        sqlx::query(
            "INSERT INTO test_balances (customer_id, tenant_id, balance)
             VALUES ($1, $2, $3)",
        )
        .bind(&customer_id)
        .bind("tenant-123")
        .bind((i * 100) as i64)
        .execute(&pool)
        .await
        .expect("Failed to insert test data");
    }

    // Compute digest twice
    let digest1 = compute_digest(&pool, "test_balances", "customer_id")
        .await
        .expect("Failed to compute digest 1");

    let digest2 = compute_digest(&pool, "test_balances", "customer_id")
        .await
        .expect("Failed to compute digest 2");

    // Digests should be identical (deterministic)
    assert_eq!(digest1, digest2, "Digest should be deterministic");

    // Clean up
    sqlx::query("DROP TABLE test_balances")
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_blue_green_swap_atomic() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    // Create live table
    create_test_projection_table(&pool, "test_balances").await;

    // Insert data into live table
    sqlx::query(
        "INSERT INTO test_balances (customer_id, tenant_id, balance)
         VALUES ('cust-old', 'tenant-123', 1000)",
    )
    .execute(&pool)
    .await
    .expect("Failed to insert into live table");

    // Create shadow table
    create_test_projection_table(&pool, "test_balances_shadow").await;

    // Insert different data into shadow table
    sqlx::query(
        "INSERT INTO test_balances_shadow (customer_id, tenant_id, balance)
         VALUES ('cust-new', 'tenant-123', 2000)",
    )
    .execute(&pool)
    .await
    .expect("Failed to insert into shadow table");

    // Perform atomic swap
    swap_tables_atomic(&pool, "test_balances")
        .await
        .expect("Failed to swap tables");

    // Verify: live table should now have shadow data
    let balance: i64 = sqlx::query_scalar(
        "SELECT balance FROM test_balances WHERE customer_id = 'cust-new'",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch balance from swapped table");

    assert_eq!(balance, 2000, "Live table should have shadow data after swap");

    // Verify: old table should exist with old data
    let old_balance: i64 = sqlx::query_scalar(
        "SELECT balance FROM test_balances_old WHERE customer_id = 'cust-old'",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch balance from old table");

    assert_eq!(old_balance, 1000, "Old table should preserve original data");

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS test_balances CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS test_balances_old CASCADE")
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_complete_rebuild_workflow() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    // Clean up from previous runs
    sqlx::query("DROP TABLE IF EXISTS test_balances CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS test_balances_shadow CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS test_balances_old CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(&pool)
        .await
        .ok();

    let projection_name = "customer_balance";
    let tenant_id = "tenant-rebuild";
    let event_count = 50;

    // Step 1: Create shadow table
    let create_ddl = r#"
        CREATE TABLE test_balances_shadow (
            customer_id VARCHAR(100) PRIMARY KEY,
            tenant_id VARCHAR(100) NOT NULL,
            balance BIGINT NOT NULL DEFAULT 0,
            event_count INT NOT NULL DEFAULT 0,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
    "#;

    create_shadow_table(&pool, "test_balances", create_ddl)
        .await
        .expect("Failed to create shadow table");

    // Step 2: Create shadow cursor table
    create_shadow_cursor_table(&pool)
        .await
        .expect("Failed to create shadow cursor table");

    // Step 3: Replay events into shadow table
    let (last_event_id, _last_event_occurred_at, events_processed) = replay_events_into_table(
        &pool,
        "test_balances_shadow",
        "projection_cursors_shadow",
        projection_name,
        tenant_id,
        event_count,
    )
    .await;

    assert_eq!(events_processed, event_count as i64);

    // Step 4: Compute digest of shadow table
    let shadow_digest = compute_digest(&pool, "test_balances_shadow", "customer_id")
        .await
        .expect("Failed to compute shadow digest");

    // Step 5: Perform atomic swap
    swap_tables_atomic(&pool, "test_balances")
        .await
        .expect("Failed to swap tables");
    swap_cursor_tables_atomic(&pool)
        .await
        .expect("Failed to swap cursor tables");

    // Step 6: Verify live table has correct data
    let live_digest = compute_digest(&pool, "test_balances", "customer_id")
        .await
        .expect("Failed to compute live digest");

    assert_eq!(
        shadow_digest, live_digest,
        "Live table should have same digest as shadow after swap"
    );

    // Verify cursor was swapped
    let cursor: (Uuid, i64) = sqlx::query_as(
        "SELECT last_event_id, events_processed FROM projection_cursors
         WHERE projection_name = $1 AND tenant_id = $2",
    )
    .bind(projection_name)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to fetch cursor");

    assert_eq!(cursor.0, last_event_id);
    assert_eq!(cursor.1, events_processed);

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS test_balances CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS test_balances_old CASCADE")
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_rebuild_determinism_two_runs() {
    let pool = get_projections_pool().await;
    run_base_migrations(&pool).await;

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS test_balances CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS test_balances_shadow CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(&pool)
        .await
        .ok();

    let projection_name = "customer_balance";
    let tenant_id = "tenant-determinism";
    let event_count = 30;

    // Helper to run a complete rebuild
    async fn run_rebuild(
        pool: &PgPool,
        projection_name: &str,
        tenant_id: &str,
        event_count: usize,
    ) -> String {
        // Create shadow table
        let create_ddl = r#"
            CREATE TABLE test_balances_shadow (
                customer_id VARCHAR(100) PRIMARY KEY,
                tenant_id VARCHAR(100) NOT NULL,
                balance BIGINT NOT NULL DEFAULT 0,
                event_count INT NOT NULL DEFAULT 0,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
        "#;

        create_shadow_table(pool, "test_balances", create_ddl)
            .await
            .expect("Failed to create shadow table");

        create_shadow_cursor_table(pool)
            .await
            .expect("Failed to create shadow cursor table");

        // Use deterministic event IDs and timestamps for reproducibility
        // In a real system, you'd replay from the same event stream
        let deterministic_timestamp =
            chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc);

        for i in 0..event_count {
            let event_id = Uuid::from_u128(i as u128); // Deterministic UUID
            let customer_id = format!("cust-{}", i % 10);

            // Use explicit updated_at for determinism
            sqlx::query(
                r#"
                INSERT INTO test_balances_shadow (customer_id, tenant_id, balance, event_count, updated_at)
                VALUES ($1, $2, $3, 1, $4)
                ON CONFLICT (customer_id)
                DO UPDATE SET
                    balance = test_balances_shadow.balance + $3,
                    event_count = test_balances_shadow.event_count + 1,
                    updated_at = $4
                "#,
            )
            .bind(&customer_id)
            .bind(tenant_id)
            .bind(100_i64)
            .bind(deterministic_timestamp)
            .execute(pool)
            .await
            .expect("Failed to apply event");

            save_shadow_cursor(pool, projection_name, tenant_id, event_id, deterministic_timestamp)
                .await
                .expect("Failed to save shadow cursor");
        }

        // Compute digest
        let digest = compute_digest(pool, "test_balances_shadow", "customer_id")
            .await
            .expect("Failed to compute digest");

        // Clean up shadow tables for next run
        drop_shadow_table(pool, "test_balances")
            .await
            .expect("Failed to drop shadow table");
        sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
            .execute(pool)
            .await
            .expect("Failed to drop shadow cursor table");

        digest
    }

    // Run rebuild twice with same event stream
    let digest1 = run_rebuild(&pool, projection_name, tenant_id, event_count).await;
    let digest2 = run_rebuild(&pool, projection_name, tenant_id, event_count).await;

    // Digests should be identical (deterministic rebuild)
    assert_eq!(
        digest1, digest2,
        "Two rebuilds with same event stream should produce identical digests"
    );

    // Clean up
    sqlx::query("DROP TABLE IF EXISTS test_balances CASCADE")
        .execute(&pool)
        .await
        .ok();
}

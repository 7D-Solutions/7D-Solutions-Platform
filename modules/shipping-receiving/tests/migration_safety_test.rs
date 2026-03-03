//! Migration Safety Tests (Phase 58 Gate A, bd-227n8)
//!
//! Proves shipping-receiving migrations can be applied on a fresh DB and that
//! the forward-fix rollback procedure works (drop all tables in reverse order).
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5454 (docker compose up -d)

use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};

// ============================================================================
// Test DB helpers
// ============================================================================

async fn connect() -> PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB")
}

// ============================================================================
// Test 1: Migrations apply cleanly on a fresh database
// ============================================================================

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;

    // Apply all migrations (must not error)
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All migrations must apply without error");

    // Verify all expected tables exist
    let expected_tables = vec![
        "shipments",
        "shipment_lines",
        "sr_events_outbox",
        "sr_processed_events",
    ];

    for table in &expected_tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("checking table {}: {}", table, e));

        assert!(exists, "Table '{}' must exist after migrations", table);
    }

    // Verify migration count
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .expect("migration count query");
    assert!(
        migration_count >= 3,
        "At least 3 successful migrations expected, got {}",
        migration_count
    );
}

// ============================================================================
// Test 2: Forward-fix rollback — drop all tables, re-apply cleanly
// ============================================================================

#[tokio::test]
#[serial]
async fn forward_fix_rollback_and_reapply() {
    let pool = connect().await;

    // Ensure migrations are applied
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("initial apply");

    // Execute full rollback (reverse dependency order)
    let rollback_statements = [
        "DROP TABLE IF EXISTS sr_carrier_requests CASCADE",
        "DROP TABLE IF EXISTS sr_shipping_doc_requests CASCADE",
        "DROP TABLE IF EXISTS rma_receipt_items CASCADE",
        "DROP TABLE IF EXISTS rma_receipts CASCADE",
        "DROP TABLE IF EXISTS inspection_routings CASCADE",
        "DROP TABLE IF EXISTS shipment_lines CASCADE",
        "DROP TABLE IF EXISTS shipments CASCADE",
        "DROP TABLE IF EXISTS sr_processed_events CASCADE",
        "DROP TABLE IF EXISTS sr_events_outbox CASCADE",
        "DROP TABLE IF EXISTS _sqlx_migrations CASCADE",
    ];

    for stmt in &rollback_statements {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("rollback statement failed: {} — {}", stmt, e));
    }

    // Verify tables are gone
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name IN \
         ('shipments', 'shipment_lines', 'sr_events_outbox', 'sr_processed_events')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        remaining, 0,
        "No shipping-receiving tables should remain after rollback"
    );

    // Re-apply all migrations — must succeed on clean slate
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-apply after rollback must succeed");

    // Verify tables are back
    for table in &["shipments", "shipment_lines", "sr_events_outbox", "sr_processed_events"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(*table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "Table '{}' must exist after re-apply", table);
    }
}

// ============================================================================
// Test 3: All data tables have tenant_id column (tenant isolation by design)
// ============================================================================

#[tokio::test]
#[serial]
async fn all_data_tables_have_tenant_id() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    // Tables that must have tenant_id for multi-tenant isolation
    let tenant_tables = vec!["shipments", "shipment_lines", "sr_events_outbox"];

    for table in &tenant_tables {
        let has_tenant: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT FROM information_schema.columns
                WHERE table_name = $1 AND column_name = 'tenant_id'
            )",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("checking tenant_id on {}: {}", table, e));

        assert!(
            has_tenant,
            "Table '{}' must have a tenant_id column for multi-tenant isolation",
            table
        );
    }
}

// ============================================================================
// Test 4: sr_processed_events has correct schema (event_id PK, no processor)
// ============================================================================

#[tokio::test]
#[serial]
async fn sr_processed_events_has_correct_schema() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    // event_id must be the primary key
    let has_pk: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT FROM information_schema.table_constraints tc
            JOIN information_schema.constraint_column_usage ccu
              ON tc.constraint_name = ccu.constraint_name
            WHERE tc.table_name = 'sr_processed_events'
              AND tc.constraint_type = 'PRIMARY KEY'
              AND ccu.column_name = 'event_id'
        )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        has_pk,
        "sr_processed_events must have event_id as PRIMARY KEY"
    );

    // processor column must NOT exist (was removed by fixed migration 3)
    let has_processor: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT FROM information_schema.columns
            WHERE table_name = 'sr_processed_events' AND column_name = 'processor'
        )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !has_processor,
        "sr_processed_events must NOT have a processor column"
    );
}

// ============================================================================
// Test 5: Shipments table constraints are intact
// ============================================================================

#[tokio::test]
#[serial]
async fn shipments_table_constraints_intact() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    // Direction constraint: only inbound/outbound allowed
    let bad_direction = sqlx::query(
        "INSERT INTO shipments (tenant_id, direction, status) \
         VALUES ($1, 'invalid_dir', 'draft')",
    )
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await;
    assert!(
        bad_direction.is_err(),
        "Invalid direction must be rejected by CHECK constraint"
    );

    // Status constraint: invalid status for direction must fail
    let bad_status = sqlx::query(
        "INSERT INTO shipments (tenant_id, direction, status) \
         VALUES ($1, 'inbound', 'picking')",
    )
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await;
    assert!(
        bad_status.is_err(),
        "Invalid status for direction must be rejected by CHECK constraint"
    );

    // Quantity constraints on shipment_lines
    let tenant_id = uuid::Uuid::new_v4();
    let ship: (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) \
         VALUES ($1, 'inbound', 'draft') RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("insert valid shipment");

    let bad_qty = sqlx::query(
        "INSERT INTO shipment_lines (tenant_id, shipment_id, qty_expected) \
         VALUES ($1, $2, -1)",
    )
    .bind(tenant_id)
    .bind(ship.0)
    .execute(&pool)
    .await;
    assert!(
        bad_qty.is_err(),
        "Negative qty_expected must be rejected by CHECK constraint"
    );
}

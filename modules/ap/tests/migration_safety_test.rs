//! Migration Safety Tests (Phase 58 Gate A, bd-mvane)
//!
//! Proves AP migrations can be applied on a fresh DB and that the
//! forward-fix rollback procedure works (drop all tables in reverse order).
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5443 (docker compose up -d)

use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};

// ============================================================================
// Test DB helpers
// ============================================================================

async fn connect() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("Failed to connect to AP test DB")
}

// ============================================================================
// Test 1: Migrations apply cleanly
// ============================================================================

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All AP migrations must apply without error");

    let expected_tables = vec![
        "events_outbox",
        "processed_events",
        "vendors",
        "purchase_orders",
        "po_lines",
        "po_status",
        "po_receipt_links",
        "vendor_bills",
        "bill_lines",
        "three_way_match",
        "ap_allocations",
        "payment_runs",
        "idempotency_keys",
        "payment_run_items",
        "payment_run_executions",
        "ap_tax_snapshots",
        "payment_terms",
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

    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .expect("migration count query");
    assert!(
        migration_count >= 14,
        "At least 14 successful migrations expected, got {}",
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
        "DROP TABLE IF EXISTS ap_tax_snapshots CASCADE",
        "DROP TABLE IF EXISTS payment_run_executions CASCADE",
        "DROP TABLE IF EXISTS payment_run_items CASCADE",
        "DROP TABLE IF EXISTS idempotency_keys CASCADE",
        "DROP TABLE IF EXISTS ap_allocations CASCADE",
        "DROP TABLE IF EXISTS payment_runs CASCADE",
        "DROP TABLE IF EXISTS three_way_match CASCADE",
        "DROP TABLE IF EXISTS bill_lines CASCADE",
        "DROP TABLE IF EXISTS vendor_bills CASCADE",
        "DROP TABLE IF EXISTS payment_terms CASCADE",
        "DROP TABLE IF EXISTS po_receipt_links CASCADE",
        "DROP TABLE IF EXISTS po_status CASCADE",
        "DROP TABLE IF EXISTS po_lines CASCADE",
        "DROP TABLE IF EXISTS purchase_orders CASCADE",
        "DROP TABLE IF EXISTS vendors CASCADE",
        "DROP TABLE IF EXISTS processed_events CASCADE",
        "DROP TABLE IF EXISTS events_outbox CASCADE",
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
         ('events_outbox','processed_events','vendors','purchase_orders',\
          'po_lines','po_status','vendor_bills','bill_lines','payment_runs',\
          'ap_allocations','idempotency_keys','payment_run_items',\
          'payment_run_executions','ap_tax_snapshots','three_way_match',\
          'po_receipt_links','payment_terms')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        remaining, 0,
        "No AP tables should remain after rollback, got {}",
        remaining
    );

    // Re-apply all migrations — must succeed on clean slate
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-apply after rollback must succeed");

    // Verify tables are back
    for table in ["vendors", "purchase_orders", "vendor_bills", "payment_runs"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table)
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

    // Tables that must have tenant_id for multi-tenant isolation.
    // Child/join tables (po_lines, bill_lines, etc.) scope via parent FK.
    let tenant_tables = vec![
        "vendors",
        "purchase_orders",
        "vendor_bills",
        "payment_runs",
        "ap_allocations",
        "ap_tax_snapshots",
        "idempotency_keys",
        "payment_terms",
    ];

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

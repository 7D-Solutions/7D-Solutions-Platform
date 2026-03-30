//! Migration Safety Tests (Phase 58 Gate A, bd-goyni)
//!
//! Proves inventory migrations can be applied on a fresh DB and that the
//! forward-fix rollback procedure works (drop all tables in reverse order).
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5442 (docker compose up -d)

use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};

// ============================================================================
// Test DB helpers
// ============================================================================

async fn connect() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require"
            .to_string()
    });
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB")
}

// ============================================================================
// Test 1: Migrations apply cleanly
// ============================================================================

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;

    // Apply all migrations
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All migrations must apply without error");

    // Verify all expected tables exist
    let expected_tables = vec![
        "items",
        "inventory_ledger",
        "inventory_layers",
        "layer_consumptions",
        "inventory_reservations",
        "item_on_hand",
        "inv_outbox",
        "inv_processed_events",
        "inv_idempotency_keys",
        "uoms",
        "item_uom_conversions",
        "inventory_lots",
        "inventory_serial_instances",
        "item_on_hand_by_status",
        "locations",
        "inv_status_transfers",
        "inv_adjustments",
        "cycle_count_tasks",
        "cycle_count_lines",
        "inv_transfers",
        "reorder_policies",
        "inventory_valuation_snapshots",
        "inventory_valuation_lines",
        "inv_low_stock_state",
        "item_revisions",
        "inv_labels",
        "inv_lot_expiry_alert_state",
        "inv_lot_genealogy",
        "item_change_history",
        "item_valuation_configs",
        "valuation_runs",
        "valuation_run_lines",
        "item_classifications",
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

    // Verify migration count in sqlx tracking table
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .expect("migration count query");
    assert!(
        migration_count >= 25,
        "At least 25 successful migrations expected, got {}",
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
    let rollback_sql = r#"
        DROP TABLE IF EXISTS item_classifications CASCADE;
        DROP TABLE IF EXISTS inv_lot_genealogy CASCADE;
        DROP TABLE IF EXISTS inv_labels CASCADE;
        DROP TABLE IF EXISTS inv_lot_expiry_alert_state CASCADE;
        DROP TABLE IF EXISTS inv_low_stock_state CASCADE;
        DROP TABLE IF EXISTS valuation_run_lines CASCADE;
        DROP TABLE IF EXISTS valuation_runs CASCADE;
        DROP TABLE IF EXISTS item_valuation_configs CASCADE;
        DROP TABLE IF EXISTS item_change_history CASCADE;
        DROP TABLE IF EXISTS item_revisions CASCADE;
        DROP TABLE IF EXISTS inventory_valuation_lines CASCADE;
        DROP TABLE IF EXISTS inventory_valuation_snapshots CASCADE;
        DROP TABLE IF EXISTS reorder_policies CASCADE;
        DROP TABLE IF EXISTS inv_transfers CASCADE;
        DROP TABLE IF EXISTS cycle_count_lines CASCADE;
        DROP TABLE IF EXISTS cycle_count_tasks CASCADE;
        DROP TYPE IF EXISTS cycle_count_status CASCADE;
        DROP TYPE IF EXISTS cycle_count_scope CASCADE;
        DROP TABLE IF EXISTS inv_adjustments CASCADE;
        DROP TABLE IF EXISTS inv_status_transfers CASCADE;
        DROP TABLE IF EXISTS locations CASCADE;
        DROP TABLE IF EXISTS item_on_hand_by_status CASCADE;
        DROP TYPE IF EXISTS inv_item_status CASCADE;
        DROP TABLE IF EXISTS inventory_serial_instances CASCADE;
        DROP TABLE IF EXISTS inventory_lots CASCADE;
        DROP TABLE IF EXISTS item_uom_conversions CASCADE;
        DROP TABLE IF EXISTS uoms CASCADE;
        DROP TABLE IF EXISTS inv_idempotency_keys CASCADE;
        DROP TABLE IF EXISTS inv_processed_events CASCADE;
        DROP TABLE IF EXISTS inv_outbox CASCADE;
        DROP TABLE IF EXISTS item_on_hand CASCADE;
        DROP TABLE IF EXISTS inventory_reservations CASCADE;
        DROP TYPE IF EXISTS inv_reservation_status CASCADE;
        DROP TABLE IF EXISTS layer_consumptions CASCADE;
        DROP TABLE IF EXISTS inventory_layers CASCADE;
        DROP TABLE IF EXISTS inventory_ledger CASCADE;
        DROP TYPE IF EXISTS inv_entry_type CASCADE;
        DROP TABLE IF EXISTS items CASCADE;
        DROP TABLE IF EXISTS _sqlx_migrations CASCADE;
    "#;

    // Execute each statement individually (sqlx doesn't support multi-statement)
    for stmt in rollback_sql.split(';') {
        let trimmed = stmt.trim();
        if !trimmed.is_empty() {
            sqlx::query(trimmed)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("rollback statement failed: {} — {}", trimmed, e));
        }
    }

    // Verify tables are gone
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name LIKE 'inv%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        remaining, 0,
        "No inventory tables should remain after rollback"
    );

    let items_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'items')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!items_exists, "items table should be gone after rollback");

    // Re-apply all migrations — must succeed on clean slate
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-apply after rollback must succeed");

    // Verify tables are back
    let items_back: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'items')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(items_back, "items table must exist after re-apply");

    let ledger_back: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'inventory_ledger')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(ledger_back, "inventory_ledger must exist after re-apply");
}

// ============================================================================
// Test 3: All tables have tenant_id column (tenant isolation by design)
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
    let tenant_tables = vec![
        "items",
        "inventory_ledger",
        "inventory_layers",
        "item_on_hand",
        "inv_outbox",
        "inv_idempotency_keys",
        "uoms",
        "inventory_lots",
        "inventory_serial_instances",
        "item_on_hand_by_status",
        "locations",
        "inv_status_transfers",
        "inv_adjustments",
        "reorder_policies",
        "inventory_valuation_snapshots",
        "inv_low_stock_state",
        "item_revisions",
        "inv_labels",
        "inv_lot_expiry_alert_state",
        "inv_lot_genealogy",
        "item_change_history",
        "item_valuation_configs",
        "valuation_runs",
        "item_classifications",
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

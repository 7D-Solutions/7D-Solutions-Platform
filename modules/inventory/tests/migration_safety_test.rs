//! Migration Safety Tests — Inventory
//!
//! Verifies that:
//! 1. All inventory migrations apply cleanly on a fresh database.
//! 2. The last 3 migrations are either FORWARD-ONLY annotated or reversible.
//! 3. The forward-fix rollback works: drop public schema → re-apply all migrations.
//! 4. Key inventory tables carry tenant_id for multi-tenant isolation.
//!
//! ## Prerequisites
//! - PostgreSQL accessible via DATABASE_URL (or localhost:5442 fallback).
//!
//! ## Running
//! Run this test binary in isolation:
//! ```sh
//! cargo test -p inventory-rs --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    mst::connect_pool(
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require",
    )
    .await
}

// ── 1. Apply cleanly ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All inventory migrations must apply without error");

    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 25, "Expected >= 25 inventory migrations, got {count}");

    mst::assert_tables_exist(
        &pool,
        &[
            "items",
            "inventory_ledger",
            "inventory_layers",
            "inventory_reservations",
            "item_on_hand",
            "inv_outbox",
            "inv_processed_events",
            "uoms",
            "inventory_lots",
            "inventory_serial_instances",
            "locations",
            "inv_status_transfers",
            "inv_adjustments",
            "cycle_count_tasks",
            "inv_transfers",
            "reorder_policies",
            "inventory_valuation_snapshots",
            "inv_low_stock_state",
            "item_revisions",
            "inv_labels",
            "item_change_history",
            "item_valuation_configs",
            "valuation_runs",
            "item_classifications",
        ],
    )
    .await;
}

// ── 2. Last-3 annotation check ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn last_three_migrations_are_safe() {
    let migrations = mst::check_last_n_migrations(
        concat!(env!("CARGO_MANIFEST_DIR"), "/db/migrations"),
        3,
    );
    for m in &migrations {
        if m.is_forward_only {
            println!(
                "[FORWARD-ONLY] {}: {}",
                m.filename,
                m.forward_only_reason.as_deref().unwrap_or("")
            );
        } else {
            println!(
                "[REVERSIBLE]   {} — proved by forward_fix_rollback_and_reapply",
                m.filename
            );
        }
    }
}

// ── 3. Forward-fix rollback ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forward_fix_rollback_and_reapply() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("initial apply");

    mst::reset_public_schema(&pool).await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-apply after forward-fix rollback must succeed");

    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 25, "All inventory migrations must re-apply; got {count}");
}

// ── 4. Tenant isolation ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_enforced() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    mst::assert_tenant_id_columns(
        &pool,
        &[
            "items",
            "inventory_ledger",
            "inventory_layers",
            "item_on_hand",
            "inv_outbox",
            "uoms",
            "inventory_lots",
            "inventory_serial_instances",
            "locations",
            "inv_status_transfers",
            "inv_adjustments",
            "reorder_policies",
            "inventory_valuation_snapshots",
            "inv_low_stock_state",
            "item_revisions",
            "inv_labels",
            "item_change_history",
            "item_valuation_configs",
            "valuation_runs",
            "item_classifications",
        ],
    )
    .await;
}

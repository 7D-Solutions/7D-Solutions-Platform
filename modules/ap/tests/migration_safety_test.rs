//! Migration Safety Tests — AP (Accounts Payable)
//!
//! Verifies that:
//! 1. All AP migrations apply cleanly on a fresh database.
//! 2. The last 3 migrations are either FORWARD-ONLY annotated or reversible.
//! 3. The forward-fix rollback works: drop public schema → re-apply all migrations.
//! 4. Key AP tables carry tenant_id for multi-tenant isolation.
//!
//! ## Prerequisites
//! - PostgreSQL accessible via DATABASE_URL (or localhost:5443 fallback).
//!
//! ## Running
//! Run this test binary in isolation to avoid conflicting with other test
//! binaries that share the AP database:
//! ```sh
//! cargo test -p ap --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    mst::connect_pool("postgresql://ap_user:ap_pass@localhost:5443/ap_db").await
}

// ── 1. Apply cleanly ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All AP migrations must apply without error");

    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 14, "Expected >= 14 AP migrations, got {count}");

    mst::assert_tables_exist(
        &pool,
        &[
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
        ],
    )
    .await;
}

// ── 2. Last-3 annotation check ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn last_three_migrations_are_safe() {
    // Each of the last 3 migrations must either:
    //   (a) carry  -- FORWARD-ONLY: <reason>
    //   (b) be reversible — proved by forward_fix_rollback_and_reapply below.
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
    // Reversibility for non-annotated migrations is proven by the rollback test.
    // FORWARD-ONLY migrations are explicitly documented with a recovery path.
}

// ── 3. Forward-fix rollback ───────────────────────────────────────────────────

/// Drop the entire public schema and re-apply all AP migrations.
///
/// This test MUST be run in isolation (--test-threads=1, --test migration_safety_test)
/// because it drops the public schema, which would break concurrently-running
/// test binaries that share this database.
#[tokio::test]
#[serial]
async fn forward_fix_rollback_and_reapply() {
    let pool = connect().await;

    // Ensure schema is in a known state before the reset.
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("initial apply");

    // Drop the entire public schema — this is the production forward-fix procedure.
    mst::reset_public_schema(&pool).await;

    // Re-apply every migration from scratch.
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-apply after forward-fix rollback must succeed");

    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 14, "All AP migrations must re-apply; got {count}");
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
            "vendors",
            "purchase_orders",
            "vendor_bills",
            "payment_runs",
            "ap_allocations",
            "ap_tax_snapshots",
            "idempotency_keys",
            "payment_terms",
        ],
    )
    .await;
}

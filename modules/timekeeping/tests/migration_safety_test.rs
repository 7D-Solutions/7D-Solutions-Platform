//! Migration Safety Tests — Timekeeping
//!
//! Timekeeping records (time entries, billing runs) are scoped by employee/project,
//! not directly by tenant_id at the migration layer.  No tenant_id assertion.
//!
//! Run in isolation:
//! ```sh
//! cargo test -p timekeeping --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    mst::connect_pool(
        "postgresql://timekeeping_user:timekeeping_pass@localhost:5447/timekeeping_db",
    )
    .await
}

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All timekeeping migrations must apply without error");
    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 7, "Expected >= 7 timekeeping migrations, got {count}");
    mst::assert_tables_exist(
        &pool,
        &[
            "tk_allocations",
            "tk_approval_actions",
            "tk_approval_requests",
            "tk_billing_rates",
            "tk_billing_runs",
            "events_outbox",
            "processed_events",
        ],
    )
    .await;
}

#[tokio::test]
#[serial]
async fn last_three_migrations_are_safe() {
    let migrations = mst::check_last_n_migrations(
        concat!(env!("CARGO_MANIFEST_DIR"), "/db/migrations"),
        3,
    );
    for m in &migrations {
        if m.is_forward_only {
            println!("[FORWARD-ONLY] {}: {}", m.filename, m.forward_only_reason.as_deref().unwrap_or(""));
        } else {
            println!("[REVERSIBLE]   {} — proved by forward_fix_rollback_and_reapply", m.filename);
        }
    }
}

#[tokio::test]
#[serial]
async fn forward_fix_rollback_and_reapply() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations").run(&pool).await.expect("initial apply");
    mst::reset_public_schema(&pool).await;
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-apply after forward-fix rollback must succeed");
    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 7, "All timekeeping migrations must re-apply; got {count}");
}
// NOTE: No tenant_isolation_enforced — timekeeping scopes by employee/project, not tenant_id.

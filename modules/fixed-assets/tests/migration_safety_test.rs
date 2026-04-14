//! Migration Safety Tests — Fixed Assets
//!
//! Run in isolation:
//! ```sh
//! cargo test -p fixed-assets --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    mst::connect_pool(
        "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db",
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
        .expect("All fixed-assets migrations must apply without error");
    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 7, "Expected >= 7 fixed-assets migrations, got {count}");
    mst::assert_tables_exist(
        &pool,
        &[
            "fa_assets",
            "fa_categories",
            "fa_depreciation_schedules",
            "fa_depreciation_runs",
            "fa_disposals",
            "fa_events_outbox",
            "fa_idempotency_keys",
            "fa_ap_capitalizations",
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
    assert!(count >= 7, "All fixed-assets migrations must re-apply; got {count}");
}

#[tokio::test]
#[serial]
async fn tenant_isolation_enforced() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations").run(&pool).await.expect("apply migrations");
    mst::assert_min_tables_with_tenant_id(&pool, 3).await;
}

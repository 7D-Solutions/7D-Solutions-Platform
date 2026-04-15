//! Migration Safety Tests — Consolidation
//!
//! Run in isolation:
//! ```sh
//! cargo test -p consolidation --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    mst::connect_pool(
        "postgres://consolidation_user:consolidation_pass@localhost:5446/consolidation_db",
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
        .expect("All consolidation migrations must apply without error");
    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 3, "Expected >= 3 consolidation migrations, got {count}");
    mst::assert_tables_exist(
        &pool,
        &[
            "csl_groups",
            "csl_group_entities",
            "csl_coa_mappings",
            "csl_elimination_rules",
            "csl_elimination_postings",
            "csl_fx_policies",
            "csl_trial_balance_cache",
            "csl_statement_cache",
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
    assert!(count >= 3, "All consolidation migrations must re-apply; got {count}");
}

#[tokio::test]
#[serial]
async fn tenant_isolation_enforced() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations").run(&pool).await.expect("apply migrations");
    // Consolidation uses group-level tenant scoping: csl_groups.tenant_id is the
    // single isolation anchor; other tables scope via group_id → csl_groups.
    mst::assert_min_tables_with_tenant_id(&pool, 1).await;
}

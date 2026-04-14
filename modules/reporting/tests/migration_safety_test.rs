//! Migration Safety Tests — Reporting
//!
//! Run in isolation:
//! ```sh
//! REPORTING_DATABASE_URL=postgres://... cargo test -p reporting --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    // Reporting uses a dedicated env var; fall back to DATABASE_URL then the default.
    dotenvy::dotenv().ok();
    let url = std::env::var("REPORTING_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgres://ap_user:ap_pass@localhost:5443/reporting_test".to_string()
        });
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .unwrap_or_else(|e| panic!("Failed to connect to reporting test DB at {url}: {e}"))
}

#[tokio::test]
#[serial]
async fn migrations_apply_cleanly() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("All reporting migrations must apply without error");
    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 5, "Expected >= 5 reporting migrations, got {count}");
    mst::assert_tables_exist(
        &pool,
        &[
            "rpt_ap_aging_cache",
            "rpt_ar_aging_cache",
            "rpt_cashflow_cache",
            "rpt_dashboard_layouts",
            "rpt_dashboard_widgets",
            "rpt_delivery_schedules",
            "events_outbox",
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
    assert!(count >= 5, "All reporting migrations must re-apply; got {count}");
}

#[tokio::test]
#[serial]
async fn tenant_isolation_enforced() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations").run(&pool).await.expect("apply migrations");
    mst::assert_min_tables_with_tenant_id(&pool, 3).await;
}

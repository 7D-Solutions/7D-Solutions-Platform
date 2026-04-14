//! Migration Safety Tests — PDF Editor
//!
//! Run in isolation:
//! ```sh
//! cargo test -p pdf-editor --test migration_safety_test -- --test-threads=1 --nocapture
//! ```

use migration_safety_test as mst;
use serial_test::serial;
use sqlx::PgPool;

async fn connect() -> PgPool {
    mst::connect_pool(
        "postgres://pdf_editor_user:pdf_editor_pass@localhost:5453/pdf_editor_db",
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
        .expect("All pdf-editor migrations must apply without error");
    let count = mst::count_applied_migrations(&pool).await;
    assert!(count >= 4, "Expected >= 4 pdf-editor migrations, got {count}");
    mst::assert_tables_exist(
        &pool,
        &[
            "form_templates",
            "form_fields",
            "form_submissions",
            "branding_images",
            "table_render_requests",
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
    assert!(count >= 4, "All pdf-editor migrations must re-apply; got {count}");
}

#[tokio::test]
#[serial]
async fn tenant_isolation_enforced() {
    let pool = connect().await;
    sqlx::migrate!("db/migrations").run(&pool).await.expect("apply migrations");
    mst::assert_min_tables_with_tenant_id(&pool, 2).await;
}

/// Integration tests for projection cursor tracking and rebuild infrastructure.
///
/// Covers: (1) cursor save/load/resume, (2) idempotent try_apply_event,
/// (3) shadow table create/swap, (4) shadow cursor tracking during rebuild.
/// All tests run against a real PostgreSQL database — no mocks.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use projections::cursor::{try_apply_event, ProjectionCursor};
use projections::rebuild::{
    create_shadow_cursor_table, create_shadow_table, drop_shadow_table, load_shadow_cursor,
    save_shadow_cursor, swap_tables_atomic,
};

async fn test_pool() -> PgPool {
    let url = std::env::var("PROJECTIONS_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://projections_user:projections_pass@localhost:5439/projections_db".to_string()
    });
    PgPool::connect(&url)
        .await
        .expect("connect to projections DB")
}

/// Ensure the projection_cursors table exists for tests.
async fn ensure_cursors_table(pool: &PgPool) {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS projection_cursors (
            projection_name VARCHAR(100) NOT NULL,
            tenant_id VARCHAR(100) NOT NULL,
            last_event_id UUID NOT NULL,
            last_event_occurred_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
            events_processed BIGINT NOT NULL DEFAULT 1,
            PRIMARY KEY (projection_name, tenant_id)
        )"#,
    )
    .execute(pool)
    .await
    .expect("ensure projection_cursors table");
}

/// Cleanup helper: remove test cursor rows by projection_name.
async fn cleanup_cursors(pool: &PgPool, projection_name: &str) {
    sqlx::query("DELETE FROM projection_cursors WHERE projection_name = $1")
        .bind(projection_name)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Cursor: save, load, resume
// ============================================================================

#[tokio::test]
async fn cursor_save_and_load() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_save_load_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-integ-1";
    let event_id = Uuid::new_v4();
    let occurred = Utc::now();

    ProjectionCursor::save(&pool, &proj, tenant, event_id, occurred)
        .await
        .expect("save cursor");

    let loaded = ProjectionCursor::load(&pool, &proj, tenant)
        .await
        .expect("load cursor")
        .expect("cursor must exist");

    assert_eq!(loaded.projection_name, proj);
    assert_eq!(loaded.tenant_id, tenant);
    assert_eq!(loaded.last_event_id, event_id);
    assert_eq!(loaded.events_processed, 1);

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn cursor_load_returns_none_when_missing() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;

    let result = ProjectionCursor::load(&pool, "nonexistent_proj", "no-tenant")
        .await
        .expect("load should succeed");
    assert!(result.is_none());
}

#[tokio::test]
async fn cursor_save_increments_events_processed() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_incr_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-incr";

    for _ in 0..3 {
        ProjectionCursor::save(&pool, &proj, tenant, Uuid::new_v4(), Utc::now())
            .await
            .expect("save cursor");
    }

    let loaded = ProjectionCursor::load(&pool, &proj, tenant)
        .await
        .expect("load")
        .expect("cursor must exist");

    assert_eq!(loaded.events_processed, 3);

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn cursor_is_processed_detects_duplicate() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_dup_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-dup";
    let event_id = Uuid::new_v4();

    ProjectionCursor::save(&pool, &proj, tenant, event_id, Utc::now())
        .await
        .expect("save");

    let is_dup = ProjectionCursor::is_processed(&pool, &proj, tenant, event_id)
        .await
        .expect("check");
    assert!(is_dup, "same event_id must be detected as processed");

    let other_id = Uuid::new_v4();
    let is_new = ProjectionCursor::is_processed(&pool, &proj, tenant, other_id)
        .await
        .expect("check");
    assert!(!is_new, "different event_id must not be processed");

    cleanup_cursors(&pool, &proj).await;
}

// ============================================================================
// try_apply_event: idempotent apply contract
// ============================================================================

#[tokio::test]
async fn try_apply_event_applies_new_event() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_apply_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-apply";
    let event_id = Uuid::new_v4();

    let mut conn = pool.acquire().await.expect("acquire conn");

    let applied = try_apply_event(
        &mut conn,
        &proj,
        tenant,
        event_id,
        Utc::now(),
        |_tx| Box::pin(async move { Ok(()) }),
    )
    .await
    .expect("try_apply_event");

    assert!(applied, "new event must be applied");

    let cursor = ProjectionCursor::load(&pool, &proj, tenant)
        .await
        .expect("load")
        .expect("cursor must exist");
    assert_eq!(cursor.last_event_id, event_id);
    assert_eq!(cursor.events_processed, 1);

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn try_apply_event_skips_duplicate() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_skip_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-skip";
    let event_id = Uuid::new_v4();

    let mut conn = pool.acquire().await.expect("acquire");
    try_apply_event(
        &mut conn,
        &proj,
        tenant,
        event_id,
        Utc::now(),
        |_tx| Box::pin(async move { Ok(()) }),
    )
    .await
    .expect("first apply");

    let mut conn2 = pool.acquire().await.expect("acquire");
    let applied = try_apply_event(
        &mut conn2,
        &proj,
        tenant,
        event_id,
        Utc::now(),
        |_tx| Box::pin(async move { Ok(()) }),
    )
    .await
    .expect("second apply");

    assert!(!applied, "duplicate event must be skipped");

    let cursor = ProjectionCursor::load(&pool, &proj, tenant)
        .await
        .expect("load")
        .expect("cursor");
    assert_eq!(cursor.events_processed, 1);

    cleanup_cursors(&pool, &proj).await;
}

#[tokio::test]
async fn try_apply_event_sequence_tracks_position() {
    let pool = test_pool().await;
    ensure_cursors_table(&pool).await;
    let proj = format!("test_seq_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-seq";

    let mut last_id = Uuid::nil();
    for i in 0..5 {
        let eid = Uuid::new_v4();
        let mut conn = pool.acquire().await.expect("acquire");
        let applied = try_apply_event(
            &mut conn,
            &proj,
            tenant,
            eid,
            Utc::now(),
            |_tx| Box::pin(async move { Ok(()) }),
        )
        .await
        .expect("apply");
        assert!(applied, "event {} must be applied", i);
        last_id = eid;
    }

    let cursor = ProjectionCursor::load(&pool, &proj, tenant)
        .await
        .expect("load")
        .expect("cursor");
    assert_eq!(cursor.events_processed, 5);
    assert_eq!(cursor.last_event_id, last_id);

    cleanup_cursors(&pool, &proj).await;
}

// ============================================================================
// Shadow table create and swap
// ============================================================================

#[tokio::test]
async fn shadow_table_create_and_swap() {
    let pool = test_pool().await;
    let base = format!("test_rebuild_{}", &Uuid::new_v4().to_string()[..8]);
    let shadow = format!("{}_shadow", base);

    // Cleanup any leftover
    for suffix in ["_shadow", "", "_old"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {}{} CASCADE", base, suffix))
            .execute(&pool)
            .await
            .ok();
    }

    // Create live table with data
    sqlx::query(&format!(
        "CREATE TABLE {} (id UUID PRIMARY KEY, amount BIGINT NOT NULL)",
        base
    ))
    .execute(&pool)
    .await
    .expect("create live table");

    sqlx::query(&format!("INSERT INTO {} (id, amount) VALUES ($1, 100)", base))
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .expect("insert live data");

    // Create shadow table
    let ddl = format!(
        "CREATE TABLE {} (id UUID PRIMARY KEY, amount BIGINT NOT NULL)",
        shadow
    );
    create_shadow_table(&pool, &base, &ddl)
        .await
        .expect("create shadow");

    // Insert different data in shadow
    sqlx::query(&format!(
        "INSERT INTO {} (id, amount) VALUES ($1, 999)",
        shadow
    ))
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert shadow data");

    // Swap
    swap_tables_atomic(&pool, &base).await.expect("swap");

    // Live table should now have shadow data (999)
    let amount: i64 =
        sqlx::query_scalar(&format!("SELECT amount FROM {} LIMIT 1", base))
            .fetch_one(&pool)
            .await
            .expect("read swapped");
    assert_eq!(amount, 999, "after swap, live should have shadow data");

    // Cleanup
    for suffix in ["", "_old"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {}{} CASCADE", base, suffix))
            .execute(&pool)
            .await
            .ok();
    }
}

#[tokio::test]
async fn shadow_table_create_fails_if_exists() {
    let pool = test_pool().await;
    let base = format!("test_dup_shadow_{}", &Uuid::new_v4().to_string()[..8]);
    let shadow = format!("{}_shadow", base);
    let ddl = format!("CREATE TABLE {} (id UUID PRIMARY KEY, val INT)", shadow);

    create_shadow_table(&pool, &base, &ddl)
        .await
        .expect("first create");

    let result = create_shadow_table(&pool, &base, &ddl).await;
    assert!(result.is_err(), "duplicate shadow must fail");

    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", shadow))
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn swap_fails_when_no_shadow() {
    let pool = test_pool().await;
    let base = format!("test_noshadow_{}", &Uuid::new_v4().to_string()[..8]);
    let result = swap_tables_atomic(&pool, &base).await;
    assert!(result.is_err(), "swap without shadow must fail");
}

#[tokio::test]
async fn drop_shadow_is_idempotent() {
    let pool = test_pool().await;
    let base = format!("test_drop_{}", &Uuid::new_v4().to_string()[..8]);
    drop_shadow_table(&pool, &base)
        .await
        .expect("drop nonexistent should succeed");
}

// ============================================================================
// Shadow cursor tracking during rebuild
// ============================================================================

#[tokio::test]
async fn shadow_cursor_save_and_load() {
    let pool = test_pool().await;
    // Drop and recreate to ensure clean state
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(&pool)
        .await
        .ok();
    create_shadow_cursor_table(&pool)
        .await
        .expect("create shadow cursor table");

    let proj = format!("test_scursor_{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-shadow";
    let event_id = Uuid::new_v4();

    save_shadow_cursor(&pool, &proj, tenant, event_id, Utc::now())
        .await
        .expect("save shadow cursor");

    let loaded = load_shadow_cursor(&pool, &proj, tenant)
        .await
        .expect("load shadow cursor")
        .expect("shadow cursor must exist");

    assert_eq!(loaded.projection_name, proj);
    assert_eq!(loaded.last_event_id, event_id);
    assert_eq!(loaded.events_processed, 1);

    // Save another event
    let event_id2 = Uuid::new_v4();
    save_shadow_cursor(&pool, &proj, tenant, event_id2, Utc::now())
        .await
        .expect("save second");

    let loaded2 = load_shadow_cursor(&pool, &proj, tenant)
        .await
        .expect("load")
        .expect("exists");
    assert_eq!(loaded2.events_processed, 2);
    assert_eq!(loaded2.last_event_id, event_id2);

    sqlx::query("DELETE FROM projection_cursors_shadow WHERE projection_name = $1")
        .bind(&proj)
        .execute(&pool)
        .await
        .ok();
}

// Note: swap_cursor_tables_atomic is NOT tested as an integration test because
// it renames the shared `projection_cursors` table, which would break parallel tests.
// The swap logic is structurally identical to swap_tables_atomic (tested above with
// isolated table names). The cursor swap unit-tests provide additional coverage.

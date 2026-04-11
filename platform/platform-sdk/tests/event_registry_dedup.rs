//! Integration tests for event registry idempotency deduplication.
//!
//! Requires a running PostgreSQL instance. Set `DATABASE_URL` to run.

use event_bus::EventEnvelope;
use platform_sdk::event_registry::RouteOutcome;
use platform_sdk::idempotency;
use platform_sdk::{EventRegistry, ModuleContext};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

async fn test_pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("DATABASE_URL not set — skipping dedup integration test");
            return None;
        }
    };
    Some(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("failed to connect to test database"),
    )
}

fn make_ctx(pool: PgPool) -> ModuleContext {
    let manifest = platform_sdk::Manifest::from_str(
        "[module]\nname = \"dedup-test\"\nversion = \"0.1.0\"",
        None,
    )
    .expect("valid minimal manifest");
    ModuleContext::new(pool, manifest, None)
}

fn make_envelope_with_id(
    event_id: Uuid,
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
) -> EventEnvelope<serde_json::Value> {
    let mut env = EventEnvelope::new(
        "tenant-dedup-test".into(),
        "dedup-test-module".into(),
        event_type.into(),
        payload,
    );
    env.event_id = event_id;
    env.schema_version = schema_version.into();
    env
}

async fn drop_table(pool: &PgPool, table: &str) {
    sqlx::query(&format!(r#"DROP TABLE IF EXISTS "{table}" CASCADE"#))
        .execute(pool)
        .await
        .expect("drop dedup table");
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestPayload {
    value: String,
}

// ──────────────────────────────────────────────────────────────────
// Test 1: ensure_dedupe_table creates the table and is idempotent
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ensure_dedupe_table_creates_and_is_idempotent() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dedup_ensure";
    drop_table(&pool, table).await;

    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("first ensure should succeed");

    let cols: Vec<(String,)> = sqlx::query_as(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_name = $1 AND table_schema = 'public' \
         ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&pool)
    .await
    .expect("column query");

    let col_names: Vec<&str> = cols.iter().map(|r| r.0.as_str()).collect();
    assert!(col_names.contains(&"event_id"), "missing event_id");
    assert!(col_names.contains(&"processed_at"), "missing processed_at");

    // Second call — idempotent
    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("second ensure should be idempotent");

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 2: check_and_mark returns true for new events, false for dupes
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn check_and_mark_new_event_returns_true() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dedup_cam_new";
    drop_table(&pool, table).await;
    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("ensure dedup table");

    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin tx");
    let is_new = idempotency::check_and_mark(&mut tx, table, event_id)
        .await
        .expect("check_and_mark");
    tx.commit().await.expect("commit");

    assert!(is_new, "brand-new event_id should return true");

    drop_table(&pool, table).await;
}

#[tokio::test]
async fn check_and_mark_duplicate_returns_false() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dedup_cam_dup";
    drop_table(&pool, table).await;
    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("ensure dedup table");

    let event_id = Uuid::new_v4();

    // First INSERT — should succeed
    let mut tx1 = pool.begin().await.expect("begin tx1");
    let first = idempotency::check_and_mark(&mut tx1, table, event_id)
        .await
        .expect("first check_and_mark");
    tx1.commit().await.expect("commit tx1");
    assert!(first, "first call should return true");

    // Second INSERT — same event_id, should be duplicate
    let mut tx2 = pool.begin().await.expect("begin tx2");
    let second = idempotency::check_and_mark(&mut tx2, table, event_id)
        .await
        .expect("second check_and_mark");
    tx2.commit().await.expect("commit tx2");
    assert!(!second, "second call with same event_id should return false");

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 3: dispatch_with_dedup processes a unique event_id once
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_with_dedup_processes_unique_event() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dedup_unique";
    drop_table(&pool, table).await;
    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("ensure dedup table");

    let call_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        move |_ctx, _env: EventEnvelope<TestPayload>| {
            let count = Arc::clone(&call_count_clone);
            async move {
                *count.lock().expect("test assertion") += 1;
                RouteOutcome::Handled
            }
        },
    );

    let event_id = Uuid::new_v4();
    let env = make_envelope_with_id(
        event_id,
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "unique-1"}),
    );
    let ctx = make_ctx(pool.clone());

    registry
        .dispatch_with_dedup(ctx, env, table)
        .await
        .expect("dispatch_with_dedup should succeed");

    assert_eq!(
        *call_count.lock().expect("test assertion"),
        1,
        "handler should be called exactly once for a unique event"
    );

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 4: dispatch_with_dedup skips duplicate event_id silently
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_with_dedup_skips_duplicate_event_id() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dedup_skip_dup";
    drop_table(&pool, table).await;
    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("ensure dedup table");

    let call_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        move |_ctx, _env: EventEnvelope<TestPayload>| {
            let count = Arc::clone(&call_count_clone);
            async move {
                *count.lock().expect("test assertion") += 1;
                RouteOutcome::Handled
            }
        },
    );

    let event_id = Uuid::new_v4();

    // First dispatch — processed
    let env1 = make_envelope_with_id(
        event_id,
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "first"}),
    );
    registry
        .dispatch_with_dedup(make_ctx(pool.clone()), env1, table)
        .await
        .expect("first dispatch should succeed");

    // Second dispatch — same event_id, must be skipped
    let env2 = make_envelope_with_id(
        event_id,
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "duplicate"}),
    );
    registry
        .dispatch_with_dedup(make_ctx(pool.clone()), env2, table)
        .await
        .expect("duplicate dispatch should return Ok (silent skip)");

    assert_eq!(
        *call_count.lock().expect("test assertion"),
        1,
        "handler must be called only once despite two dispatches with the same event_id"
    );

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 5: handler failure rolls back the dedup entry (event retryable)
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_with_dedup_rolls_back_on_handler_failure() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dedup_rollback";
    drop_table(&pool, table).await;
    idempotency::ensure_dedupe_table(&pool, table)
        .await
        .expect("ensure dedup table");

    let call_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        move |_ctx, _env: EventEnvelope<TestPayload>| {
            let count = Arc::clone(&call_count_clone);
            async move {
                *count.lock().expect("test assertion") += 1;
                RouteOutcome::Retried
            }
        },
    );

    let event_id = Uuid::new_v4();

    // First dispatch — handler fails; dedup INSERT must be rolled back
    let env1 = make_envelope_with_id(
        event_id,
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "will-fail"}),
    );
    let result = registry
        .dispatch_with_dedup(make_ctx(pool.clone()), env1, table)
        .await;
    assert!(result.is_err(), "handler failure must propagate as Err");

    // Verify dedup entry was rolled back (table should be empty)
    let row: (i64,) =
        sqlx::query_as(&format!(r#"SELECT COUNT(*) FROM "{table}""#))
            .fetch_one(&pool)
            .await
            .expect("count query");
    assert_eq!(row.0, 0, "dedup entry must be rolled back on handler failure");

    // Second dispatch with the same event_id — must NOT be treated as duplicate
    // (dedup was rolled back, so the event should be processed again)
    let env2 = make_envelope_with_id(
        event_id,
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "will-fail-again"}),
    );
    let _ = registry
        .dispatch_with_dedup(make_ctx(pool.clone()), env2, table)
        .await;

    assert_eq!(
        *call_count.lock().expect("test assertion"),
        2,
        "event must be dispatched again after dedup rollback"
    );

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 6: STANDARD_DEDUPE_DDL constant sanity check (no real DB needed)
// ──────────────────────────────────────────────────────────────────

#[test]
fn standard_dedupe_ddl_contains_required_columns() {
    let ddl = platform_sdk::STANDARD_DEDUPE_DDL;
    assert!(ddl.contains("event_id"), "DDL must have event_id");
    assert!(ddl.contains("processed_at"), "DDL must have processed_at");
    assert!(ddl.contains("PRIMARY KEY"), "DDL must have PRIMARY KEY on event_id");
    assert!(ddl.contains("IF NOT EXISTS"), "DDL must use IF NOT EXISTS");
    assert!(ddl.contains("{table}"), "DDL must use {{table}} placeholder");
}

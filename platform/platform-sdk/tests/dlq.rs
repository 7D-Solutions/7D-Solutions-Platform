//! Integration tests for the platform SDK dead-letter queue.
//!
//! Requires a running PostgreSQL instance. Set `DATABASE_URL` to run.

use std::sync::{Arc, Mutex};

use event_bus::EventEnvelope;
use platform_sdk::dlq;
use platform_sdk::event_registry::RouteOutcome;
use platform_sdk::{EventRegistry, ModuleContext};
use sqlx::PgPool;

/// Connect to the test database or skip.
async fn test_pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("DATABASE_URL not set — skipping DLQ integration test");
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
        "[module]\nname = \"dlq-test\"\nversion = \"0.1.0\"",
        None,
    )
    .expect("valid minimal manifest");
    ModuleContext::new(pool, manifest, None)
}

fn make_envelope(
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
) -> EventEnvelope<serde_json::Value> {
    let mut env = EventEnvelope::new(
        "tenant-dlq-test".into(),
        "dlq-test-module".into(),
        event_type.into(),
        payload,
    );
    env.schema_version = schema_version.into();
    env
}

async fn drop_table(pool: &PgPool, table: &str) {
    sqlx::query(&format!(r#"DROP TABLE IF EXISTS "{table}" CASCADE"#))
        .execute(pool)
        .await
        .expect("drop DLQ table");
}

// ──────────────────────────────────────────────────────────────────
// Test 1: ensure_dlq_table creates the table and is idempotent
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ensure_dlq_table_creates_and_is_idempotent() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dlq_ensure";
    drop_table(&pool, table).await;

    // First call — creates the table
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("first ensure should succeed");

    // Verify expected columns
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
    assert!(col_names.contains(&"id"), "missing id");
    assert!(col_names.contains(&"event_id"), "missing event_id");
    assert!(col_names.contains(&"event_type"), "missing event_type");
    assert!(
        col_names.contains(&"schema_version"),
        "missing schema_version"
    );
    assert!(col_names.contains(&"tenant_id"), "missing tenant_id");
    assert!(col_names.contains(&"payload"), "missing payload");
    assert!(
        col_names.contains(&"error_message"),
        "missing error_message"
    );
    assert!(col_names.contains(&"retry_count"), "missing retry_count");
    assert!(col_names.contains(&"failed_at"), "missing failed_at");
    assert!(col_names.contains(&"replayed_at"), "missing replayed_at");

    // Second call — idempotent, no error
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("second ensure should be idempotent");

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 2: dispatch_with_dlq writes an entry when the handler fails
// ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestPayload {
    value: String,
}

#[tokio::test]
async fn dispatch_with_dlq_inserts_entry_on_handler_failure() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dlq_insert";
    drop_table(&pool, table).await;
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("ensure DLQ table");

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        |_ctx, _env: EventEnvelope<TestPayload>| async { RouteOutcome::DeadLettered },
    );

    let env = make_envelope(
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "test-123"}),
    );
    let ctx = make_ctx(pool.clone());

    // dispatch_with_dlq must return Ok even though the handler failed
    registry
        .dispatch_with_dlq(ctx, env, table)
        .await
        .expect("dispatch_with_dlq should return Ok after writing to DLQ");

    // Verify the DLQ entry was written
    let entries = dlq::list_dlq_entries(&pool, table, 10, false)
        .await
        .expect("list_dlq_entries");

    assert_eq!(entries.len(), 1, "expected exactly one DLQ entry");
    let entry = &entries[0];
    assert_eq!(entry.event_type, "order.placed");
    assert_eq!(entry.schema_version, "1.0.0");
    assert_eq!(entry.tenant_id, "tenant-dlq-test");
    assert!(
        entry.error_message.contains("intentional test failure"),
        "error_message should contain the handler error: {}",
        entry.error_message
    );
    assert_eq!(entry.retry_count, 0);
    assert!(entry.replayed_at.is_none(), "not yet replayed");
    assert_eq!(
        entry.payload,
        serde_json::json!({"value": "test-123"}),
        "payload should match original"
    );

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 3: dispatch_with_dlq does NOT write to DLQ on handler success
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_with_dlq_no_entry_on_success() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dlq_no_entry";
    drop_table(&pool, table).await;
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("ensure DLQ table");

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        |_ctx, _env: EventEnvelope<TestPayload>| async { RouteOutcome::Handled },
    );

    let env = make_envelope("order.placed", "1.0.0", serde_json::json!({"value": "ok"}));
    let ctx = make_ctx(pool.clone());

    registry
        .dispatch_with_dlq(ctx, env, table)
        .await
        .expect("should succeed");

    let entries = dlq::list_dlq_entries(&pool, table, 10, false)
        .await
        .expect("list");

    assert!(entries.is_empty(), "DLQ must be empty on handler success");

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 4: replay_dlq_entry returns the envelope and marks replayed
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_dlq_entry_returns_envelope_and_marks_replayed() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dlq_replay";
    drop_table(&pool, table).await;
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("ensure DLQ table");

    // Manually insert a DLQ entry (simulates a prior failure)
    let original_event_id = uuid::Uuid::new_v4();
    let entry_id = dlq::write_dlq_entry(
        &pool,
        table,
        original_event_id,
        "invoice.opened",
        "2.0.0",
        "tenant-replay",
        &serde_json::json!({"invoice_id": "inv-99"}),
        "simulated handler error",
        0,
    )
    .await
    .expect("write DLQ entry");

    // Replay
    let env_opt = dlq::replay_dlq_entry(&pool, table, entry_id)
        .await
        .expect("replay should succeed");

    let env = env_opt.expect("entry should be found");
    assert_eq!(
        env.event_id, original_event_id,
        "event_id must be preserved"
    );
    assert_eq!(env.event_type, "invoice.opened");
    assert_eq!(env.schema_version, "2.0.0");
    assert_eq!(env.tenant_id, "tenant-replay");
    assert_eq!(env.payload, serde_json::json!({"invoice_id": "inv-99"}));

    // The entry should now be marked as replayed
    let entries = dlq::list_dlq_entries(&pool, table, 10, true)
        .await
        .expect("list including replayed");
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].replayed_at.is_some(),
        "replayed_at must be set after replay"
    );

    // Pending-only list should now be empty
    let pending = dlq::list_dlq_entries(&pool, table, 10, false)
        .await
        .expect("list pending");
    assert!(pending.is_empty(), "no pending entries after replay");

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 5: replay re-dispatches successfully via the registry
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_dlq_entry_and_redispatch_calls_handler() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dlq_redispatch";
    drop_table(&pool, table).await;
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("ensure DLQ table");

    // Write a DLQ entry
    let original_event_id = uuid::Uuid::new_v4();
    let entry_id = dlq::write_dlq_entry(
        &pool,
        table,
        original_event_id,
        "order.shipped",
        "1.0.0",
        "tenant-redispatch",
        &serde_json::json!({"value": "shipment-7"}),
        "prior failure",
        0,
    )
    .await
    .expect("write DLQ entry");

    // Registry that succeeds on re-dispatch
    let called: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let called_clone = Arc::clone(&called);

    let registry = EventRegistry::new().on(
        "order.shipped",
        "1.0.0",
        move |_ctx, env: EventEnvelope<TestPayload>| {
            let called = Arc::clone(&called_clone);
            async move {
                *called.lock().expect("test assertion") = Some(env.payload.value.clone());
                RouteOutcome::Handled
            }
        },
    );

    // Replay retrieves the envelope
    let env = dlq::replay_dlq_entry(&pool, table, entry_id)
        .await
        .expect("replay")
        .expect("entry found");

    // Re-dispatch via the registry
    let ctx = make_ctx(pool.clone());
    let outcome = registry.dispatch(ctx, env).await;
    assert_eq!(outcome, RouteOutcome::Handled, "re-dispatch should succeed");

    // Handler was called with the replayed payload
    let result = called.lock().expect("test assertion").clone();
    assert_eq!(
        result,
        Some("shipment-7".into()),
        "handler should receive the replayed payload"
    );

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 6: replay_dlq_entry returns None for unknown id
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_dlq_entry_returns_none_for_missing_id() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_dlq_missing";
    drop_table(&pool, table).await;
    dlq::ensure_dlq_table(&pool, table)
        .await
        .expect("ensure DLQ table");

    let result = dlq::replay_dlq_entry(&pool, table, 99999)
        .await
        .expect("should not error");

    assert!(result.is_none(), "missing id should return None");

    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 7: STANDARD_DLQ_DDL constant sanity check (no real DB needed)
// ──────────────────────────────────────────────────────────────────

#[test]
fn standard_dlq_ddl_contains_required_columns() {
    let ddl = platform_sdk::STANDARD_DLQ_DDL;
    assert!(ddl.contains("event_id"), "DDL must have event_id");
    assert!(ddl.contains("event_type"), "DDL must have event_type");
    assert!(
        ddl.contains("schema_version"),
        "DDL must have schema_version"
    );
    assert!(ddl.contains("payload"), "DDL must have payload");
    assert!(ddl.contains("error_message"), "DDL must have error_message");
    assert!(ddl.contains("retry_count"), "DDL must have retry_count");
    assert!(ddl.contains("failed_at"), "DDL must have failed_at");
    assert!(ddl.contains("replayed_at"), "DDL must have replayed_at");
    assert!(ddl.contains("IF NOT EXISTS"), "DDL must use IF NOT EXISTS");
    assert!(
        ddl.contains("{table}"),
        "DDL must use {{table}} placeholder"
    );
}

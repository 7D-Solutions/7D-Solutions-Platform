//! Integration tests for EventRegistry schema_version routing and RouteOutcome.
//!
//! Tests 1–2 are pure unit tests (no DB required).
//! Tests 3–4 require a running PostgreSQL instance — set `DATABASE_URL` to run.

use event_bus::EventEnvelope;
use platform_sdk::dlq;
use platform_sdk::event_registry::RouteOutcome;
use platform_sdk::{EventRegistry, ModuleContext};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestPayload {
    value: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

fn make_envelope(
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
) -> EventEnvelope<serde_json::Value> {
    let mut env = EventEnvelope::new(
        "tenant-test".into(),
        "test-module".into(),
        event_type.into(),
        payload,
    );
    env.schema_version = schema_version.into();
    env
}

fn make_envelope_with_id(
    event_id: Uuid,
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
) -> EventEnvelope<serde_json::Value> {
    let mut env = make_envelope(event_type, schema_version, payload);
    env.event_id = event_id;
    env
}

fn make_ctx_lazy() -> ModuleContext {
    let pool = sqlx::PgPool::connect_lazy("postgres://dummy:dummy@localhost/dummy")
        .expect("connect_lazy does not establish a connection");
    let manifest =
        platform_sdk::Manifest::from_str("[module]\nname = \"test\"\nversion = \"0.1.0\"", None)
            .expect("valid minimal manifest");
    ModuleContext::new(pool, manifest, None)
}

async fn test_pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("DATABASE_URL not set — skipping DB-backed integration test");
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

fn make_ctx_real(pool: PgPool) -> ModuleContext {
    let manifest = platform_sdk::Manifest::from_str(
        "[module]\nname = \"event-registry-test\"\nversion = \"0.1.0\"",
        None,
    )
    .expect("valid minimal manifest");
    ModuleContext::new(pool, manifest, None)
}

async fn drop_table(pool: &PgPool, table: &str) {
    sqlx::query(&format!(r#"DROP TABLE IF EXISTS "{table}" CASCADE"#))
        .execute(pool)
        .await
        .expect("drop table");
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 1: Two handlers registered for same event_type with different versions —
//         correct handler fires for each version.
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn version_specific_handlers_dispatch_to_correct_version() {
    let called: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let called_v1 = Arc::clone(&called);
    let called_v2 = Arc::clone(&called);

    let registry = EventRegistry::new()
        .on(
            "order.placed",
            "1.0.0",
            move |_ctx, _env: EventEnvelope<TestPayload>| {
                let c = Arc::clone(&called_v1);
                async move {
                    c.lock().expect("lock").push("v1".into());
                    RouteOutcome::Handled
                }
            },
        )
        .on(
            "order.placed",
            "2.0.0",
            move |_ctx, _env: EventEnvelope<TestPayload>| {
                let c = Arc::clone(&called_v2);
                async move {
                    c.lock().expect("lock").push("v2".into());
                    RouteOutcome::Handled
                }
            },
        );

    // Dispatch v1 then v2 — each must route to the matching handler only.
    let outcome_v1 = registry
        .dispatch(
            make_ctx_lazy(),
            make_envelope("order.placed", "1.0.0", serde_json::json!({"value": "a"})),
        )
        .await;
    let outcome_v2 = registry
        .dispatch(
            make_ctx_lazy(),
            make_envelope("order.placed", "2.0.0", serde_json::json!({"value": "b"})),
        )
        .await;

    assert_eq!(outcome_v1, RouteOutcome::Handled, "v1 must return Handled");
    assert_eq!(outcome_v2, RouteOutcome::Handled, "v2 must return Handled");

    let log = called.lock().expect("lock").clone();
    assert_eq!(
        log,
        vec!["v1", "v2"],
        "each version must fire its own handler"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 2: on_any_version — handler fires as fallback for unregistered versions;
//         exact version wins when registered.
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn on_any_version_fires_for_unregistered_versions() {
    let called: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let c_exact = Arc::clone(&called);
    let c_any = Arc::clone(&called);

    let registry = EventRegistry::new()
        .on(
            "order.placed",
            "1.0.0",
            move |_ctx, _env: EventEnvelope<TestPayload>| {
                let c = Arc::clone(&c_exact);
                async move {
                    c.lock().expect("lock").push("exact-v1".into());
                    RouteOutcome::Handled
                }
            },
        )
        .on_any_version(
            "order.placed",
            move |_ctx, _env: EventEnvelope<TestPayload>| {
                let c = Arc::clone(&c_any);
                async move {
                    c.lock().expect("lock").push("wildcard".into());
                    RouteOutcome::Handled
                }
            },
        );

    // v1.0.0 → exact handler
    registry
        .dispatch(
            make_ctx_lazy(),
            make_envelope("order.placed", "1.0.0", serde_json::json!({"value": "a"})),
        )
        .await;

    // v2.0.0 — no exact handler → wildcard fires
    let outcome_fallback = registry
        .dispatch(
            make_ctx_lazy(),
            make_envelope("order.placed", "2.0.0", serde_json::json!({"value": "b"})),
        )
        .await;

    // completely unknown event — no wildcard for this type → Unknown
    let outcome_unknown = registry
        .dispatch(
            make_ctx_lazy(),
            make_envelope("invoice.opened", "1.0.0", serde_json::json!({"value": "c"})),
        )
        .await;

    assert_eq!(
        outcome_fallback,
        RouteOutcome::Handled,
        "wildcard handler must fire for unregistered version"
    );
    assert_eq!(
        outcome_unknown,
        RouteOutcome::Unknown,
        "unknown event_type with no wildcard must return Unknown"
    );

    let log = called.lock().expect("lock").clone();
    assert_eq!(
        log,
        vec!["exact-v1", "wildcard"],
        "v1 goes to exact handler; v2 goes to wildcard"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 3: Handler returns Skipped — event is not retried and not dead-lettered.
//         Verified against a real DLQ table (no rows written).
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn skipped_outcome_does_not_write_dlq_and_does_not_error() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let dlq_table = "sdk_test_er_skipped_dlq";
    drop_table(&pool, dlq_table).await;
    dlq::ensure_dlq_table(&pool, dlq_table)
        .await
        .expect("ensure DLQ table");

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        |_ctx, _env: EventEnvelope<TestPayload>| async { RouteOutcome::Skipped },
    );

    let env = make_envelope_with_id(
        Uuid::new_v4(),
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "skip-me"}),
    );
    let ctx = make_ctx_real(pool.clone());

    // dispatch_with_dlq must return Ok (not an error — no retry)
    let result = registry.dispatch_with_dlq(ctx, env, dlq_table).await;
    assert!(result.is_ok(), "Skipped must not return an error");

    // DLQ table must be empty — nothing was dead-lettered
    let row: (i64,) = sqlx::query_as(&format!(r#"SELECT COUNT(*) FROM "{dlq_table}""#))
        .fetch_one(&pool)
        .await
        .expect("count query");
    assert_eq!(row.0, 0, "Skipped outcome must not write a DLQ entry");

    drop_table(&pool, dlq_table).await;
}

// ──────────────────────────────────────────────────────────────────────────────
// Test 4: Handler returns DeadLettered — event is written to the DLQ table.
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dead_lettered_outcome_writes_to_dlq_table() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let dlq_table = "sdk_test_er_dead_lettered_dlq";
    drop_table(&pool, dlq_table).await;
    dlq::ensure_dlq_table(&pool, dlq_table)
        .await
        .expect("ensure DLQ table");

    let registry = EventRegistry::new().on(
        "order.placed",
        "1.0.0",
        |_ctx, _env: EventEnvelope<TestPayload>| async { RouteOutcome::DeadLettered },
    );

    let event_id = Uuid::new_v4();
    let env = make_envelope_with_id(
        event_id,
        "order.placed",
        "1.0.0",
        serde_json::json!({"value": "dead-letter-me"}),
    );
    let ctx = make_ctx_real(pool.clone());

    // dispatch_with_dlq must return Ok (message is acked — sent to DLQ)
    let result = registry.dispatch_with_dlq(ctx, env, dlq_table).await;
    assert!(
        result.is_ok(),
        "DeadLettered must return Ok (message acknowledged)"
    );

    // Exactly one DLQ entry must exist with the correct event_id.
    // Note: PostgreSQL has no MIN(uuid) aggregate, so we use two queries.
    let count: (i64,) = sqlx::query_as(&format!(r#"SELECT COUNT(*) FROM "{dlq_table}""#))
        .fetch_one(&pool)
        .await
        .expect("count query");
    assert_eq!(count.0, 1, "exactly one DLQ entry must be written");

    let stored_id: (uuid::Uuid,) =
        sqlx::query_as(&format!(r#"SELECT event_id FROM "{dlq_table}" LIMIT 1"#))
            .fetch_one(&pool)
            .await
            .expect("event_id query");
    assert_eq!(
        stored_id.0, event_id,
        "DLQ entry must record the original event_id"
    );

    drop_table(&pool, dlq_table).await;
}

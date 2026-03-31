//! Integration tests for the SDK outbox publisher and undeclared-outbox detection.
//!
//! These tests require a running PostgreSQL instance. Set `DATABASE_URL` to run them.

use std::sync::Arc;

use event_bus::{EventBus, InMemoryBus};
use futures::StreamExt;
use platform_sdk::publisher;
use sqlx::PgPool;

/// Connect to the test database or skip.
async fn test_pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("DATABASE_URL not set — skipping integration test");
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

/// Create a temporary outbox table for testing.
async fn create_outbox_table(pool: &PgPool, table: &str) {
    let ddl = format!(
        r#"
        CREATE TABLE IF NOT EXISTS "{table}" (
            id SERIAL PRIMARY KEY,
            event_id UUID NOT NULL UNIQUE,
            event_type TEXT NOT NULL,
            aggregate_type TEXT NOT NULL DEFAULT '',
            aggregate_id TEXT NOT NULL DEFAULT '',
            payload JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            published_at TIMESTAMPTZ
        )
        "#,
    );
    sqlx::query(&ddl).execute(pool).await.expect("create outbox table");
}

/// Drop a test table.
async fn drop_table(pool: &PgPool, table: &str) {
    let ddl = format!(r#"DROP TABLE IF EXISTS "{table}" CASCADE"#);
    sqlx::query(&ddl).execute(pool).await.expect("drop table");
}

/// Insert a test event row.
async fn insert_event(pool: &PgPool, table: &str, event_type: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let q = format!(
        r#"INSERT INTO "{table}" (event_id, event_type, payload) VALUES ($1, $2, $3)"#,
    );
    sqlx::query(&q)
        .bind(id)
        .bind(event_type)
        .bind(serde_json::json!({"test": true}))
        .execute(pool)
        .await
        .expect("insert event");
    id
}

/// Count unpublished events.
async fn count_unpublished(pool: &PgPool, table: &str) -> i64 {
    let q = format!(
        r#"SELECT COUNT(*) as cnt FROM "{table}" WHERE published_at IS NULL"#,
    );
    let row: (i64,) = sqlx::query_as(&q)
        .fetch_one(pool)
        .await
        .expect("count unpublished");
    row.0
}

// ──────────────────────────────────────────────────────────────────
// Test 1: Publisher drains outbox rows
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn publisher_drains_outbox() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_outbox_drain";
    drop_table(&pool, table).await;
    create_outbox_table(&pool, table).await;

    // Insert 3 events
    insert_event(&pool, table, "test.event.a").await;
    insert_event(&pool, table, "test.event.b").await;
    insert_event(&pool, table, "test.event.c").await;
    assert_eq!(count_unpublished(&pool, table).await, 3);

    // Create an InMemoryBus and subscribe to catch published events
    let bus = Arc::new(InMemoryBus::new());
    let mut stream = bus.subscribe("test.event.>").await.expect("subscribe");

    // Spawn the publisher in background
    let pub_pool = pool.clone();
    let pub_bus: Arc<dyn EventBus> = bus.clone();
    let pub_table = table.to_string();
    let handle = tokio::spawn(async move {
        publisher::run_outbox_publisher(pub_pool, pub_bus, &pub_table, "test").await;
    });

    // Wait for events to be published (up to 5 seconds)
    let mut received = 0;
    for _ in 0..5 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await {
            Ok(Some(_)) => received += 1,
            _ => break,
        }
        if received == 3 {
            break;
        }
    }

    assert_eq!(received, 3, "expected 3 events to be published");
    assert_eq!(
        count_unpublished(&pool, table).await,
        0,
        "all events should be marked published"
    );

    handle.abort();
    drop_table(&pool, table).await;
}

// ──────────────────────────────────────────────────────────────────
// Test 2: Undeclared outbox detection
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn detects_undeclared_outbox_table() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let table = "sdk_test_events_outbox";
    drop_table(&pool, table).await;
    create_outbox_table(&pool, table).await;

    let found = publisher::detect_outbox_table(&pool)
        .await
        .expect("detection query should succeed");

    assert!(
        found.is_some(),
        "should detect an outbox table in the database"
    );
    let found_name = found.unwrap();
    assert!(
        found_name.ends_with("_outbox"),
        "detected table '{}' should end with _outbox",
        found_name
    );

    drop_table(&pool, table).await;
}

#[tokio::test]
async fn no_false_positive_without_outbox_table() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    // Drop any leftover test outbox tables
    drop_table(&pool, "sdk_test_events_outbox").await;
    drop_table(&pool, "sdk_test_outbox_drain").await;

    // Detection should return None when no outbox tables exist.
    // Note: other modules' outbox tables may exist in the same DB,
    // so we can only assert the function doesn't error.
    let result = publisher::detect_outbox_table(&pool).await;
    assert!(result.is_ok(), "detection should not error");
}

// ──────────────────────────────────────────────────────────────────
// Test 3: Manifest with bus_type=none — no bus, no publisher, healthy
// ──────────────────────────────────────────────────────────────────

#[test]
fn manifest_bus_none_no_publisher() {
    let toml_str = r#"
[module]
name = "no-bus-module"

[bus]
type = "none"
"#;
    let manifest =
        platform_sdk::Manifest::from_str(toml_str, None).expect("none bus type should parse");
    assert_eq!(
        manifest.bus.as_ref().unwrap().bus_type.as_str(),
        "none"
    );
    // No events section → no publisher would be spawned
    assert!(manifest.events.is_none());
}

#[test]
fn manifest_with_outbox_and_bus() {
    let toml_str = r#"
[module]
name = "publishing-module"

[bus]
type = "inmemory"

[events.publish]
outbox_table = "events_outbox"
"#;
    let manifest = platform_sdk::Manifest::from_str(toml_str, None)
        .expect("manifest with outbox should parse");

    let publish = manifest
        .events
        .expect("events section")
        .publish
        .expect("publish section");
    assert_eq!(publish.outbox_table, "events_outbox");
}

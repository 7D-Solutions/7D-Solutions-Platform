//! E2E tests for consumer-side envelope validation (bd-ew4yy).
//!
//! Proves:
//! 1. Forged envelope (empty required fields) → rejected before handler, DLQ entry with "poison"
//! 2. Malformed payload (not a valid EventEnvelope) → rejected at decode, DLQ entry with "poison"
//! 3. Wildcard subjects rejected by validation (NATS blocks wildcard publish at protocol level,
//!    so we verify the validation function directly — the consumer calls this on every message)
//! 4. Handler is NEVER invoked for any rejected message (no side effects)

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, stream};
use event_bus::consumer_retry::RetryConfig;
use event_bus::EventEnvelope;
use event_consumer::{
    validate_incoming, ConsumerConfig, EventRouter, JetStreamConsumer, RegistryBuilder,
    ValidationError,
};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn audit_db_url() -> String {
    std::env::var("EVENT_CONSUMER_DATABASE_URL")
        .or_else(|_| std::env::var("AUDIT_DATABASE_URL"))
        .or_else(|_| std::env::var("PLATFORM_AUDIT_DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://audit_user:audit_pass@localhost:5440/audit_db?sslmode=disable".to_string()
        })
}

async fn get_pool() -> Option<PgPool> {
    use sqlx::postgres::PgPoolOptions;
    match PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&audit_db_url())
        .await
    {
        Ok(pool) => {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                Some(pool)
            } else {
                eprintln!("skipping: audit DB not responding");
                None
            }
        }
        Err(e) => {
            eprintln!("skipping: audit DB unavailable ({e})");
            None
        }
    }
}

async fn run_migrations(pool: &PgPool) {
    let dedupe_sql = include_str!("../../platform/event-consumer/sql/event_dedupe.sql");
    let dlq_sql = include_str!("../../platform/event-consumer/sql/event_dlq.sql");
    sqlx::raw_sql(dedupe_sql)
        .execute(pool)
        .await
        .expect("event_dedupe migration");
    sqlx::raw_sql(dlq_sql)
        .execute(pool)
        .await
        .expect("event_dlq migration");
    // Backfill payload_hash column if the table predates the column addition.
    sqlx::raw_sql(
        "ALTER TABLE event_dlq ADD COLUMN IF NOT EXISTS payload_hash TEXT NOT NULL DEFAULT ''",
    )
    .execute(pool)
    .await
    .expect("payload_hash backfill");
}

async fn nats_client() -> Option<async_nats::Client> {
    let url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://platform:dev-nats-token@localhost:4222".to_string());
    match event_bus::connect_nats(&url).await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("skipping: NATS unavailable ({e})");
            None
        }
    }
}

async fn create_test_stream(js: &jetstream::Context, tag: &str) -> String {
    let stream_name = format!("TEST_SECVAL_{tag}");
    let _ = js.delete_stream(&stream_name).await;
    js.create_stream(stream::Config {
        name: stream_name.clone(),
        subjects: vec![format!("test.secval.{tag}.>")],
        max_age: Duration::from_secs(60),
        ..Default::default()
    })
    .await
    .expect("create test stream");
    stream_name
}

fn fast_retry() -> RetryConfig {
    RetryConfig {
        max_attempts: 1,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(10),
    }
}

/// Build a consumer with a handler that increments a counter.
/// If the counter is > 0 after processing, the handler was called (bad for rejection tests).
fn build_consumer_with_spy(
    nats: async_nats::Client,
    stream_name: &str,
    tag: &str,
    pool: PgPool,
) -> (JetStreamConsumer, Arc<AtomicU32>) {
    let handler_calls = Arc::new(AtomicU32::new(0));
    let calls_clone = handler_calls.clone();

    let registry = RegistryBuilder::new()
        .register("test.item_created", "1.0.0", move |_ctx, _payload| {
            let c = calls_clone.clone();
            async move {
                c.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        })
        .build();

    let router = EventRouter::new(registry);
    let config = ConsumerConfig {
        stream_name: stream_name.to_string(),
        consumer_name: format!("test-secval-{tag}"),
        filter_subject: format!("test.secval.{tag}.>"),
        retry_config: fast_retry(),
    };

    let consumer = JetStreamConsumer::new(nats, config, router, pool);
    (consumer, handler_calls)
}

// ---------------------------------------------------------------------------
// Test 1: Forged envelope with empty tenant_id → DLQ poison, handler NOT called
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forged_envelope_empty_tenant_rejected_to_dlq() {
    let Some(pool) = get_pool().await else { return };
    run_migrations(&pool).await;
    let Some(nats) = nats_client().await else {
        return;
    };
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.secval.{tag}.events.item");

    let (consumer, handler_calls) =
        build_consumer_with_spy(nats.clone(), &stream_name, &tag, pool.clone());
    let health = consumer.health();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Publish a forged envelope with empty tenant_id.
    let forged_event_id = Uuid::new_v4();
    let forged = EventEnvelope::with_event_id(
        forged_event_id,
        String::new(), // empty tenant_id
        "attacker-module".to_string(),
        "test.item_created".to_string(),
        serde_json::json!({"item_id": 1}),
    );
    let data = serde_json::to_vec(&forged).expect("serialize");
    js.publish(subject.clone(), data.into())
        .await
        .expect("publish")
        .await
        .expect("ack");

    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Handler must NOT have been called.
    assert_eq!(
        handler_calls.load(Ordering::Relaxed),
        0,
        "handler must NOT be called for forged envelope"
    );

    // DLQ counter must be incremented.
    let snap = health.snapshot();
    assert!(
        snap.messages_dlq >= 1,
        "expected DLQ entry, got dlq={}",
        snap.messages_dlq
    );

    // Verify DLQ row in real Postgres.
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT failure_kind, error_message FROM event_dlq WHERE event_id = $1",
    )
    .bind(forged_event_id)
    .fetch_optional(&pool)
    .await
    .expect("DLQ query");

    let (kind, msg) = row.expect("DLQ entry must exist for forged envelope");
    assert_eq!(kind, "poison", "failure_kind must be 'poison'");
    assert!(
        msg.contains("tenant_id"),
        "error_message should mention tenant_id: {msg}"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;

    // Cleanup DLQ entry.
    sqlx::query("DELETE FROM event_dlq WHERE event_id = $1")
        .bind(forged_event_id)
        .execute(&pool)
        .await
        .ok();
}

// ---------------------------------------------------------------------------
// Test 2: Completely malformed JSON → decode failure → DLQ poison
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_json_rejected_to_dlq() {
    let Some(pool) = get_pool().await else { return };
    run_migrations(&pool).await;
    let Some(nats) = nats_client().await else {
        return;
    };
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.secval.{tag}.events.item");

    let (consumer, handler_calls) =
        build_consumer_with_spy(nats.clone(), &stream_name, &tag, pool.clone());
    let health = consumer.health();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Publish raw garbage that is NOT a valid EventEnvelope.
    let garbage = serde_json::json!({
        "not_an_envelope": true,
        "random_field": "attacker_data",
    });
    let data = serde_json::to_vec(&garbage).expect("serialize");
    js.publish(subject.clone(), data.into())
        .await
        .expect("publish")
        .await
        .expect("ack");

    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Handler must NOT have been called.
    assert_eq!(
        handler_calls.load(Ordering::Relaxed),
        0,
        "handler must NOT be called for malformed JSON"
    );

    // DLQ counter must be incremented.
    let snap = health.snapshot();
    assert!(
        snap.messages_dlq >= 1,
        "expected DLQ entry, got dlq={}",
        snap.messages_dlq
    );

    // Verify at least one DLQ entry with "poison" kind exists for this subject.
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT failure_kind FROM event_dlq WHERE subject = $1 AND failure_kind = 'poison' LIMIT 1",
    )
    .bind(&subject)
    .fetch_optional(&pool)
    .await
    .expect("DLQ query");
    assert!(
        row.is_some(),
        "DLQ entry must exist for malformed JSON on subject {subject}"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;

    // Cleanup DLQ entries for this subject.
    sqlx::query("DELETE FROM event_dlq WHERE subject = $1")
        .bind(&subject)
        .execute(&pool)
        .await
        .ok();
}

// ---------------------------------------------------------------------------
// Test 3: Forged envelope with nil event_id → DLQ poison
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forged_envelope_nil_event_id_rejected_to_dlq() {
    let Some(pool) = get_pool().await else { return };
    run_migrations(&pool).await;
    let Some(nats) = nats_client().await else {
        return;
    };
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.secval.{tag}.events.item");

    let (consumer, handler_calls) =
        build_consumer_with_spy(nats.clone(), &stream_name, &tag, pool.clone());
    let health = consumer.health();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Publish an envelope with nil UUID event_id.
    let forged = EventEnvelope::with_event_id(
        Uuid::nil(),
        "tenant-test".to_string(),
        "attacker-module".to_string(),
        "test.item_created".to_string(),
        serde_json::json!({"item_id": 1}),
    );
    let data = serde_json::to_vec(&forged).expect("serialize");
    js.publish(subject.clone(), data.into())
        .await
        .expect("publish")
        .await
        .expect("ack");

    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_secs(3)).await;

    assert_eq!(
        handler_calls.load(Ordering::Relaxed),
        0,
        "handler must NOT be called for nil event_id"
    );

    let snap = health.snapshot();
    assert!(
        snap.messages_dlq >= 1,
        "expected DLQ entry for nil event_id"
    );

    // nil UUID DLQ entry — the consumer uses the nil UUID as event_id in the DLQ.
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT failure_kind, error_message FROM event_dlq WHERE event_id = $1",
    )
    .bind(Uuid::nil())
    .fetch_optional(&pool)
    .await
    .expect("DLQ query");

    let (kind, msg) = row.expect("DLQ entry must exist for nil event_id");
    assert_eq!(kind, "poison");
    assert!(
        msg.contains("event_id"),
        "error should mention event_id: {msg}"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;

    sqlx::query("DELETE FROM event_dlq WHERE event_id = $1")
        .bind(Uuid::nil())
        .execute(&pool)
        .await
        .ok();
}

// ---------------------------------------------------------------------------
// Test 4: Wildcard subjects rejected by validation
// ---------------------------------------------------------------------------
// NATS blocks publishing to subjects with * or > at the protocol level.
// The consumer-side validate_incoming() is defense-in-depth.
// We verify the validation function directly — it runs on every message.

#[tokio::test]
async fn wildcard_subjects_rejected_by_validation() {
    let good_envelope = EventEnvelope::new(
        "tenant-1".to_string(),
        "inventory".to_string(),
        "inventory.item_issued".to_string(),
        serde_json::json!({"key": "value"}),
    );

    // Star wildcard
    let err =
        validate_incoming(&good_envelope, "inventory.*").expect_err("wildcard * must be rejected");
    assert!(
        matches!(err, ValidationError::Subject(_)),
        "expected Subject error, got: {err}"
    );
    assert!(err.to_string().contains("wildcard '*'"));

    // Greater-than wildcard
    let err =
        validate_incoming(&good_envelope, "inventory.>").expect_err("wildcard > must be rejected");
    assert!(
        matches!(err, ValidationError::Subject(_)),
        "expected Subject error, got: {err}"
    );
    assert!(err.to_string().contains("wildcard '>'"));

    // Star in middle of subject
    let err = validate_incoming(&good_envelope, "inv*ntory.item")
        .expect_err("embedded * must be rejected");
    assert!(matches!(err, ValidationError::Subject(_)));

    // Greater-than in middle
    let err = validate_incoming(&good_envelope, "inventory>item")
        .expect_err("embedded > must be rejected");
    assert!(matches!(err, ValidationError::Subject(_)));

    // Leading dot
    let err = validate_incoming(&good_envelope, ".inventory.item")
        .expect_err("leading dot must be rejected");
    assert!(matches!(err, ValidationError::Subject(_)));

    // Empty subject
    let err = validate_incoming(&good_envelope, "").expect_err("empty subject must be rejected");
    assert!(matches!(err, ValidationError::Subject(_)));

    // Valid subject must pass
    assert!(validate_incoming(&good_envelope, "inventory.item_issued").is_ok());
}

// ---------------------------------------------------------------------------
// Test 5: Forged envelope with bad schema_version → DLQ poison
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forged_envelope_bad_schema_version_rejected_to_dlq() {
    let Some(pool) = get_pool().await else { return };
    run_migrations(&pool).await;
    let Some(nats) = nats_client().await else {
        return;
    };
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.secval.{tag}.events.item");

    let (consumer, handler_calls) =
        build_consumer_with_spy(nats.clone(), &stream_name, &tag, pool.clone());
    let health = consumer.health();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Publish envelope with invalid schema_version.
    let bad_event_id = Uuid::new_v4();
    let mut forged = EventEnvelope::with_event_id(
        bad_event_id,
        "tenant-test".to_string(),
        "attacker".to_string(),
        "test.item_created".to_string(),
        serde_json::json!({"item_id": 1}),
    );
    forged.schema_version = "not-semver".to_string();

    let data = serde_json::to_vec(&forged).expect("serialize");
    js.publish(subject.clone(), data.into())
        .await
        .expect("publish")
        .await
        .expect("ack");

    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_secs(3)).await;

    assert_eq!(
        handler_calls.load(Ordering::Relaxed),
        0,
        "handler must NOT be called for bad schema_version"
    );

    let snap = health.snapshot();
    assert!(
        snap.messages_dlq >= 1,
        "expected DLQ entry for bad schema_version"
    );

    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT failure_kind, error_message FROM event_dlq WHERE event_id = $1",
    )
    .bind(bad_event_id)
    .fetch_optional(&pool)
    .await
    .expect("DLQ query");

    let (kind, msg) = row.expect("DLQ entry must exist for bad schema_version");
    assert_eq!(kind, "poison");
    assert!(
        msg.contains("schema_version"),
        "error should mention schema_version: {msg}"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;

    sqlx::query("DELETE FROM event_dlq WHERE event_id = $1")
        .bind(bad_event_id)
        .execute(&pool)
        .await
        .ok();
}

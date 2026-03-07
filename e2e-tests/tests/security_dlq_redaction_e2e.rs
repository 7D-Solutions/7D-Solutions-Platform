//! E2E tests for DLQ payload redaction (bd-2k8v5).
//!
//! Proves:
//! 1. When a message with sensitive data (password, api_key, secret, credit_card)
//!    fails processing and lands in the DLQ, the stored payload contains only
//!    redacted values — no raw secrets at rest.
//! 2. Non-sensitive keys are preserved intact.
//! 3. The payload_hash is present (SHA-256, 64 hex chars).

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, stream};
use event_bus::consumer_retry::RetryConfig;
use event_bus::EventEnvelope;
use event_consumer::{
    ConsumerConfig, EventRouter, HandlerError, JetStreamConsumer, RegistryBuilder,
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
    let stream_name = format!("TEST_DLQREDACT_{tag}");
    let _ = js.delete_stream(&stream_name).await;
    js.create_stream(stream::Config {
        name: stream_name.clone(),
        subjects: vec![format!("test.dlqredact.{tag}.>")],
        max_age: Duration::from_secs(60),
        ..Default::default()
    })
    .await
    .expect("create test stream");
    stream_name
}

// Known secret values — these must NEVER appear in stored DLQ payloads.
const SECRET_PASSWORD: &str = "SuperSecret123!";
const SECRET_API_KEY: &str = "sk-live-xxxxxxxxxxxx";
const SECRET_VALUE: &str = "my-secret-sauce";
const SECRET_CARD: &str = "4111111111111111";

// Non-sensitive values — these SHOULD be preserved.
const SAFE_ORDER_ID: &str = "ORD-999";

// ---------------------------------------------------------------------------
// Test: Handler failure with sensitive payload → DLQ contains only redacted data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dlq_payload_redacted_no_raw_secrets() {
    let Some(pool) = get_pool().await else { return };
    run_migrations(&pool).await;
    let Some(nats) = nats_client().await else { return };
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.dlqredact.{tag}.events.sensitive");

    // Build consumer with a handler that always fails (transient).
    // max_attempts=1 so it exhausts immediately and writes to DLQ.
    let handler_calls = Arc::new(AtomicU32::new(0));
    let calls_clone = handler_calls.clone();

    let registry = RegistryBuilder::new()
        .register(
            "test.sensitive_event",
            "1.0.0",
            move |_ctx, _payload| {
                let c = calls_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::Relaxed);
                    Err(HandlerError::Transient(
                        "intentional failure for DLQ redaction test".to_string(),
                    ))
                }
            },
        )
        .build();

    let router = EventRouter::new(registry);
    let config = ConsumerConfig {
        stream_name: stream_name.clone(),
        consumer_name: format!("test-dlqredact-{tag}"),
        filter_subject: format!("test.dlqredact.{tag}.>"),
        retry_config: RetryConfig {
            max_attempts: 1,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(10),
        },
    };

    let consumer = JetStreamConsumer::new(nats.clone(), config, router, pool.clone());
    let health = consumer.health();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Publish an envelope with sensitive data in the payload.
    let event_id = Uuid::new_v4();
    let sensitive_payload = serde_json::json!({
        "order_id": SAFE_ORDER_ID,
        "password": SECRET_PASSWORD,
        "api_key": SECRET_API_KEY,
        "secret": SECRET_VALUE,
        "credit_card": SECRET_CARD,
        "amount": 99.99,
        "nested": {
            "authorization": "Bearer top-secret-token",
            "safe_field": "visible"
        }
    });

    let envelope = EventEnvelope::with_event_id(
        event_id,
        format!("test-{}", Uuid::new_v4()),
        "test-module".to_string(),
        "test.sensitive_event".to_string(),
        sensitive_payload,
    );

    let data = serde_json::to_vec(&envelope).expect("serialize");
    js.publish(subject.clone(), data.into())
        .await
        .expect("publish")
        .await
        .expect("ack");

    // Run consumer until it processes the message.
    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Handler was called (at least once).
    assert!(
        handler_calls.load(Ordering::Relaxed) >= 1,
        "handler should have been called at least once"
    );

    // DLQ counter must show at least one entry.
    let snap = health.snapshot();
    assert!(
        snap.messages_dlq >= 1,
        "expected DLQ entry after handler failure, got dlq={}",
        snap.messages_dlq
    );

    // Query the DLQ table for the stored payload.
    let row = sqlx::query_as::<_, (serde_json::Value, String, String)>(
        "SELECT payload, payload_hash, failure_kind FROM event_dlq WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&pool)
    .await
    .expect("DLQ query");

    let (stored_payload, payload_hash, failure_kind) =
        row.expect("DLQ entry must exist for failed event");

    // -- Acceptance criterion: DLQ row exists with redacted payload --
    assert_eq!(failure_kind, "retryable");

    // Serialize the stored payload to a string for substring checks.
    let stored_str = stored_payload.to_string();

    // -- Acceptance criterion: No raw secret substrings in stored payload --
    assert!(
        !stored_str.contains(SECRET_PASSWORD),
        "stored payload must NOT contain raw password: {stored_str}"
    );
    assert!(
        !stored_str.contains(SECRET_API_KEY),
        "stored payload must NOT contain raw api_key: {stored_str}"
    );
    assert!(
        !stored_str.contains(SECRET_VALUE),
        "stored payload must NOT contain raw secret: {stored_str}"
    );
    assert!(
        !stored_str.contains(SECRET_CARD),
        "stored payload must NOT contain raw credit_card: {stored_str}"
    );
    assert!(
        !stored_str.contains("top-secret-token"),
        "stored payload must NOT contain raw authorization value: {stored_str}"
    );

    // Sensitive keys must be present but values must be "[REDACTED]".
    // The stored payload is the full serialized envelope, so sensitive keys
    // are inside the "payload" sub-object.
    let inner = &stored_payload["payload"];
    assert_eq!(
        inner["password"], "[REDACTED]",
        "password must be redacted"
    );
    assert_eq!(
        inner["api_key"], "[REDACTED]",
        "api_key must be redacted"
    );
    assert_eq!(
        inner["secret"], "[REDACTED]",
        "secret must be redacted"
    );
    assert_eq!(
        inner["credit_card"], "[REDACTED]",
        "credit_card must be redacted"
    );
    assert_eq!(
        inner["nested"]["authorization"], "[REDACTED]",
        "nested authorization must be redacted"
    );

    // Non-sensitive fields must be preserved.
    assert_eq!(
        inner["order_id"], SAFE_ORDER_ID,
        "order_id must be preserved"
    );
    assert_eq!(inner["amount"], 99.99, "amount must be preserved");
    assert_eq!(
        inner["nested"]["safe_field"], "visible",
        "nested safe_field must be preserved"
    );

    // Payload hash must be a valid SHA-256 hex string (64 chars).
    assert_eq!(
        payload_hash.len(),
        64,
        "payload_hash must be 64 hex chars, got {}",
        payload_hash.len()
    );
    assert!(
        payload_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "payload_hash must be hex: {payload_hash}"
    );

    // Shutdown consumer and cleanup.
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;

    sqlx::query("DELETE FROM event_dlq WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM event_dedupe WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();
}

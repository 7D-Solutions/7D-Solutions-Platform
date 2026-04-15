//! Stress test: Event bus flood — 500 events prove zero data loss and DLQ correctness
//!
//! Proves that under a moderate burst of 520 messages (500 valid + 20 invalid),
//! the JetStream consumer processes every valid event exactly once, routes all
//! invalid events to the DLQ, and loses nothing.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- event_bus_flood_jetstream_e2e --nocapture
//! ```

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, stream};
use event_bus::consumer_retry::RetryConfig;
use event_bus::EventEnvelope;
use event_consumer::{ConsumerConfig, EventRouter, JetStreamConsumer, RegistryBuilder};
use sqlx::PgPool;
use uuid::Uuid;

const VALID_COUNT: usize = 500;
const INVALID_COUNT: usize = 20;
const TOTAL_COUNT: usize = VALID_COUNT + INVALID_COUNT;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn audit_db_url() -> String {
    std::env::var("EVENT_CONSUMER_DATABASE_URL")
        .or_else(|_| std::env::var("AUDIT_DATABASE_URL"))
        .or_else(|_| std::env::var("PLATFORM_AUDIT_DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://audit_user:audit_pass@localhost:5440/audit_db".to_string()
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
    let dedupe_sql = include_str!("../../../platform/event-consumer/sql/event_dedupe.sql");
    let dlq_sql = include_str!("../../../platform/event-consumer/sql/event_dlq.sql");
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
    let stream_name = format!("TEST_FLOOD_{tag}");
    let _ = js.delete_stream(&stream_name).await;
    js.create_stream(stream::Config {
        name: stream_name.clone(),
        subjects: vec![format!("stress.flood.{tag}.>")],
        max_age: Duration::from_secs(120),
        ..Default::default()
    })
    .await
    .expect("create test stream");
    stream_name
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn event_bus_flood_jetstream_e2e() {
    let Some(pool) = get_pool().await else { return };
    run_migrations(&pool).await;
    let Some(nats) = nats_client().await else {
        return;
    };
    let js = jetstream::new(nats.clone());

    let tenant_id = Uuid::new_v4();
    let tag = tenant_id.simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("stress.flood.{tag}.events");

    // Track how many times the handler is called.
    let handler_calls = Arc::new(AtomicU32::new(0));
    let calls_clone = handler_calls.clone();

    let registry = RegistryBuilder::new()
        .register("stress.flood_event", "1.0.0", move |_ctx, _payload| {
            let c = calls_clone.clone();
            async move {
                c.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        })
        .build();

    let router = EventRouter::new(registry);
    let config = ConsumerConfig {
        stream_name: stream_name.clone(),
        consumer_name: format!("test-flood-{tag}"),
        filter_subject: format!("stress.flood.{tag}.>"),
        retry_config: RetryConfig {
            max_attempts: 1,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(10),
        },
    };

    let consumer = JetStreamConsumer::new(nats.clone(), config, router, pool.clone());
    let health = consumer.health();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // --- Generate event IDs upfront so we can verify them later ---
    let valid_event_ids: Vec<Uuid> = (0..VALID_COUNT).map(|_| Uuid::new_v4()).collect();
    let invalid_event_ids: Vec<Uuid> = (0..INVALID_COUNT).map(|_| Uuid::new_v4()).collect();

    // --- Flood publish: 500 valid + 20 invalid ---
    println!("Publishing {VALID_COUNT} valid + {INVALID_COUNT} invalid messages...");

    for (i, &event_id) in valid_event_ids.iter().enumerate() {
        let envelope = EventEnvelope::with_event_id(
            event_id,
            tenant_id.to_string(),
            "stress-test".to_string(),
            "stress.flood_event".to_string(),
            serde_json::json!({"seq": i, "tenant_id": tenant_id.to_string()}),
        );
        let data = serde_json::to_vec(&envelope).expect("serialize valid");
        js.publish(subject.clone(), data.into())
            .await
            .expect("publish valid")
            .await
            .expect("ack valid");
    }

    for (i, &event_id) in invalid_event_ids.iter().enumerate() {
        // Invalid: schema_version is not valid semver -> validation rejects -> DLQ Poison
        let mut envelope = EventEnvelope::with_event_id(
            event_id,
            tenant_id.to_string(),
            "stress-test".to_string(),
            "stress.flood_event".to_string(),
            serde_json::json!({"seq": VALID_COUNT + i, "invalid": true}),
        );
        envelope.schema_version = "bad".to_string();
        let data = serde_json::to_vec(&envelope).expect("serialize invalid");
        js.publish(subject.clone(), data.into())
            .await
            .expect("publish invalid")
            .await
            .expect("ack invalid");
    }

    println!("All {TOTAL_COUNT} messages published. Starting consumer...");

    // --- Start consumer ---
    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    // --- Poll until all messages are processed or timeout ---
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let snap = health.snapshot();
        let total_resolved = snap.messages_processed + snap.messages_skipped + snap.messages_dlq;
        if total_resolved >= TOTAL_COUNT as u64 {
            println!(
                "All messages resolved: processed={}, skipped={}, dlq={}",
                snap.messages_processed, snap.messages_skipped, snap.messages_dlq
            );
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "Timeout: only {total_resolved}/{TOTAL_COUNT} resolved \
                 (processed={}, skipped={}, dlq={})",
                snap.messages_processed, snap.messages_skipped, snap.messages_dlq
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // --- Assertions on health counters ---
    let snap = health.snapshot();

    assert_eq!(
        snap.messages_processed, VALID_COUNT as u64,
        "expected {VALID_COUNT} processed, got {}",
        snap.messages_processed
    );
    assert_eq!(
        snap.messages_dlq, INVALID_COUNT as u64,
        "expected {INVALID_COUNT} DLQ entries, got {}",
        snap.messages_dlq
    );
    assert_eq!(
        snap.messages_skipped, 0,
        "expected 0 skipped (no duplicates), got {}",
        snap.messages_skipped
    );

    // --- Verify handler was called exactly VALID_COUNT times ---
    assert_eq!(
        handler_calls.load(Ordering::Relaxed),
        VALID_COUNT as u32,
        "handler must be called exactly {VALID_COUNT} times"
    );

    // --- Verify dedupe table: all valid event_ids present (exactly-once) ---
    let dedupe_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM event_dedupe WHERE subject = $1")
            .bind(&subject)
            .fetch_one(&pool)
            .await
            .expect("dedupe count query");

    assert_eq!(
        dedupe_count, VALID_COUNT as i64,
        "event_dedupe must have exactly {VALID_COUNT} rows for our subject, got {dedupe_count}"
    );

    // --- Verify DLQ table: exactly INVALID_COUNT entries with failure_kind=poison ---
    let dlq_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM event_dlq WHERE subject = $1")
            .bind(&subject)
            .fetch_one(&pool)
            .await
            .expect("DLQ count query");

    assert_eq!(
        dlq_count, INVALID_COUNT as i64,
        "event_dlq must have exactly {INVALID_COUNT} rows, got {dlq_count}"
    );

    // All DLQ entries should be poison (bad schema_version).
    let dlq_poison_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM event_dlq WHERE subject = $1 AND failure_kind = 'poison'",
    )
    .bind(&subject)
    .fetch_one(&pool)
    .await
    .expect("DLQ poison count query");

    assert_eq!(
        dlq_poison_count, INVALID_COUNT as i64,
        "all DLQ entries must be poison, got {dlq_poison_count} poison out of {dlq_count}"
    );

    // --- No duplicate event_ids across dedupe + DLQ ---
    let valid_in_dedupe =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM event_dedupe WHERE event_id = ANY($1)")
            .bind(&valid_event_ids)
            .fetch_one(&pool)
            .await
            .expect("valid in dedupe query");

    assert_eq!(
        valid_in_dedupe, VALID_COUNT as i64,
        "all {VALID_COUNT} valid event_ids must be in event_dedupe, found {valid_in_dedupe}"
    );

    let invalid_in_dlq =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM event_dlq WHERE event_id = ANY($1)")
            .bind(&invalid_event_ids)
            .fetch_one(&pool)
            .await
            .expect("invalid in DLQ query");

    assert_eq!(
        invalid_in_dlq, INVALID_COUNT as i64,
        "all {INVALID_COUNT} invalid event_ids must be in event_dlq, found {invalid_in_dlq}"
    );

    println!("PASS: {VALID_COUNT} valid processed, {INVALID_COUNT} invalid in DLQ, zero loss, zero duplicates");

    // --- Shutdown and cleanup ---
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;

    sqlx::query("DELETE FROM event_dedupe WHERE subject = $1")
        .bind(&subject)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM event_dlq WHERE subject = $1")
        .bind(&subject)
        .execute(&pool)
        .await
        .ok();
}

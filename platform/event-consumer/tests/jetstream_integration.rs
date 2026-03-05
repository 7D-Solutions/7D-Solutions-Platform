//! Integration tests for JetStream consumer manager.
//! Requires real NATS (localhost:4222) and Postgres (audit_db, port 5440).

mod helpers;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_nats::jetstream::{self, stream};
use event_bus::consumer_retry::RetryConfig;
use event_bus::EventEnvelope;
use event_consumer::{
    ConsumerConfig, HandlerError, JetStreamConsumer, RegistryBuilder, EventRouter,
};
use uuid::Uuid;

/// Connect to NATS with platform auth (matches docker-compose dev config).
async fn nats_client() -> async_nats::Client {
    let url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://platform:dev-nats-token@localhost:4222".to_string());
    event_bus::connect_nats(&url)
        .await
        .expect("NATS must be running on localhost:4222")
}

/// Create a uniquely-named stream for test isolation.
async fn create_test_stream(
    js: &jetstream::Context,
    suffix: &str,
) -> String {
    let stream_name = format!("TEST_EC_{}", suffix);
    let subject_prefix = format!("test.ec.{}", suffix);

    // Delete if exists from prior run.
    let _ = js.delete_stream(&stream_name).await;

    js.create_stream(stream::Config {
        name: stream_name.clone(),
        subjects: vec![format!("{}.>", subject_prefix)],
        max_age: Duration::from_secs(60),
        ..Default::default()
    })
    .await
    .expect("create test stream");

    stream_name
}

/// Publish an EventEnvelope to JetStream.
async fn publish_envelope(
    js: &jetstream::Context,
    subject: &str,
    envelope: &EventEnvelope<serde_json::Value>,
) {
    let data = serde_json::to_vec(envelope).expect("serialize envelope");
    js.publish(subject.to_string(), data.into())
        .await
        .expect("publish")
        .await
        .expect("ack from server");
}

fn fast_retry() -> RetryConfig {
    RetryConfig {
        max_attempts: 2,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(20),
    }
}

#[tokio::test]
async fn consume_and_ack_after_successful_handler() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;
    let nats = nats_client().await;
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.ec.{}.events.item", tag);

    // Track handler calls.
    let called = Arc::new(Mutex::new(false));
    let called_clone = called.clone();

    let registry = RegistryBuilder::new()
        .register("test.item_created", "1.0.0", move |_ctx, _payload| {
            let c = called_clone.clone();
            async move {
                *c.lock().unwrap() = true;
                Ok(())
            }
        })
        .build();

    let router = EventRouter::new(registry);
    let config = ConsumerConfig {
        stream_name: stream_name.clone(),
        consumer_name: format!("test-consumer-{tag}"),
        filter_subject: format!("test.ec.{tag}.>"),
        retry_config: fast_retry(),
    };

    let consumer = JetStreamConsumer::new(nats.clone(), config, router, pool);
    let health = consumer.health();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Publish a valid envelope.
    let envelope = EventEnvelope::new(
        "tenant-test".to_string(),
        "test-module".to_string(),
        "test.item_created".to_string(),
        serde_json::json!({"item_id": 1}),
    );
    publish_envelope(&js, &subject, &envelope).await;

    // Run consumer in background.
    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });

    // Wait for processing.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify handler was called.
    assert!(*called.lock().unwrap(), "handler must be called");

    // Verify health.
    let snap = health.snapshot();
    assert!(snap.messages_processed >= 1, "processed={}", snap.messages_processed);
    assert!(snap.is_running, "consumer should be running");

    // Shutdown.
    let _ = shutdown_tx.send(true);
    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("consumer should stop")
        .expect("task should not panic");
    assert!(result.is_ok(), "consumer.run() should succeed");

    // Cleanup stream.
    let _ = js.delete_stream(&stream_name).await;
}

#[tokio::test]
async fn retry_then_dlq_on_transient_failure() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;
    let nats = nats_client().await;
    let js = jetstream::new(nats.clone());

    let tag = Uuid::new_v4().simple().to_string();
    let stream_name = create_test_stream(&js, &tag).await;
    let subject = format!("test.ec.{}.events.fail", tag);

    // Handler always fails with transient error.
    let attempt_count = Arc::new(Mutex::new(0u32));
    let attempt_clone = attempt_count.clone();

    let registry = RegistryBuilder::new()
        .register("test.always_fails", "1.0.0", move |_ctx, _payload| {
            let c = attempt_clone.clone();
            async move {
                *c.lock().unwrap() += 1;
                Err(HandlerError::Transient("simulated timeout".into()))
            }
        })
        .build();

    let router = EventRouter::new(registry);
    let config = ConsumerConfig {
        stream_name: stream_name.clone(),
        consumer_name: format!("test-consumer-{tag}"),
        filter_subject: format!("test.ec.{tag}.>"),
        retry_config: fast_retry(),
    };

    let consumer = JetStreamConsumer::new(nats.clone(), config, router, pool.clone());
    let health = consumer.health();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let event_id = Uuid::new_v4();
    let envelope = EventEnvelope::with_event_id(
        event_id,
        "tenant-test".to_string(),
        "test-module".to_string(),
        "test.always_fails".to_string(),
        serde_json::json!({"fail": true}),
    );
    publish_envelope(&js, &subject, &envelope).await;

    let handle = tokio::spawn(async move { consumer.run(shutdown_rx).await });
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Handler should be called max_attempts times (2).
    let attempts = *attempt_count.lock().unwrap();
    assert_eq!(attempts, 2, "expected 2 attempts, got {attempts}");

    // Should land in DLQ.
    let snap = health.snapshot();
    assert!(snap.messages_dlq >= 1, "dlq={}", snap.messages_dlq);

    // Verify DLQ entry in Postgres.
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT error_message FROM event_dlq WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(&pool)
    .await
    .expect("DLQ query");
    assert!(row.is_some(), "DLQ entry must exist for event_id={event_id}");

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let _ = js.delete_stream(&stream_name).await;
}

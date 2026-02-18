//! Reporting ingestion framework.
//!
//! Provides the infrastructure for subscribing to domain events and
//! populating reporting cache tables idempotently.
//!
//! ## Design
//!
//! - **[`StreamHandler`]**: trait that each downstream bead implements to
//!   process a specific event type (e.g. AR aging, GL trial balance).
//! - **[`IngestConsumer`]**: wraps a `StreamHandler` with checkpoint-based
//!   idempotency: skips events whose `event_id` was the last recorded
//!   checkpoint, then saves the checkpoint after a successful handler call.
//! - **[`start_consumer`]**: spawns a background `tokio::task` that subscribes
//!   to a NATS subject and drives `IngestConsumer::process_message` for each
//!   incoming message.
//!
//! ## Idempotency guarantee
//!
//! Two layers protect against duplicate cache rows:
//! 1. **Framework layer**: `IngestConsumer` skips the event if its `event_id`
//!    matches `last_event_id` in the checkpoint (fast path for re-deliveries
//!    of the most-recent event).
//! 2. **Handler layer**: each `StreamHandler` implementation must use
//!    `ON CONFLICT DO UPDATE` / `DO NOTHING` on its target cache table's
//!    unique constraint, ensuring replays from a checkpoint reset are safe.

pub mod checkpoints;
pub mod runner;

use std::sync::Arc;

use async_trait::async_trait;
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use sqlx::PgPool;

// ── StreamHandler trait ───────────────────────────────────────────────────────

/// A handler for a specific event stream.
///
/// Downstream beads implement this trait to ingest domain events into
/// reporting cache tables. Each implementation must be idempotent: calling
/// `handle` with the same `event_id` and `payload` multiple times must
/// produce the same cache state (use `ON CONFLICT DO UPDATE/DO NOTHING`).
#[async_trait]
pub trait StreamHandler: Send + Sync {
    /// Process one event.
    ///
    /// * `pool`      – reporting DB pool (read-cache writes only)
    /// * `tenant_id` – tenant scope extracted from the EventEnvelope
    /// * `event_id`  – UUID string from the EventEnvelope (idempotency key)
    /// * `payload`   – raw `serde_json::Value` of the EventEnvelope `data`/`payload` field
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error>;
}

// ── IngestConsumer ────────────────────────────────────────────────────────────

/// Wraps a [`StreamHandler`] with checkpoint-based idempotency.
///
/// On each message:
/// 1. Parses the EventEnvelope envelope to extract `tenant_id` and `event_id`.
/// 2. Checks the checkpoint: if `event_id == last_event_id`, skips the handler.
/// 3. Calls the handler.
/// 4. Saves the checkpoint with the processed `event_id`.
pub struct IngestConsumer {
    /// Logical name of this consumer (used as checkpoint key).
    name: Arc<str>,
    pool: PgPool,
    handler: Arc<dyn StreamHandler>,
}

impl IngestConsumer {
    /// Create a new consumer.
    ///
    /// * `name`    – unique consumer name (e.g. `"reporting.ar_aging"`)
    /// * `pool`    – reporting DB pool
    /// * `handler` – the event-specific processing logic
    pub fn new(name: impl Into<Arc<str>>, pool: PgPool, handler: Arc<dyn StreamHandler>) -> Self {
        Self {
            name: name.into(),
            pool,
            handler,
        }
    }

    /// Process a single [`BusMessage`] with idempotency protection.
    ///
    /// Extracts the EventEnvelope fields, applies the checkpoint gate,
    /// calls the handler, and saves the updated checkpoint.
    ///
    /// Returns `Ok(true)` if the event was processed, `Ok(false)` if skipped.
    pub async fn process_message(&self, msg: &BusMessage) -> Result<bool, anyhow::Error> {
        // Parse the EventEnvelope wrapper (works with both AR and Payments envelopes)
        let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)
            .map_err(|e| anyhow::anyhow!("Failed to parse envelope on {}: {}", msg.subject, e))?;

        let event_id = envelope
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing event_id in envelope on {}", msg.subject))?;

        let tenant_id = envelope
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("Missing tenant_id in envelope on {}", msg.subject)
            })?;

        // Checkpoint fast-path: skip if this is the last event we processed
        if checkpoints::is_processed(&self.pool, &self.name, tenant_id, event_id).await? {
            tracing::debug!(
                consumer = %self.name,
                tenant_id,
                event_id,
                subject = %msg.subject,
                "Skipping already-processed event"
            );
            return Ok(false);
        }

        // Extract payload — accept both "data" (AR/GL envelopes) and "payload" (Payments)
        let payload = envelope
            .get("data")
            .or_else(|| envelope.get("payload"))
            .unwrap_or(&serde_json::Value::Null);

        // Call the handler
        self.handler
            .handle(&self.pool, tenant_id, event_id, payload)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Handler '{}' failed for event {} on {}: {}",
                    self.name,
                    event_id,
                    msg.subject,
                    e
                )
            })?;

        // Save checkpoint
        checkpoints::save(&self.pool, &self.name, tenant_id, 0, event_id).await?;

        tracing::info!(
            consumer = %self.name,
            tenant_id,
            event_id,
            subject = %msg.subject,
            "Event ingested and checkpoint saved"
        );

        Ok(true)
    }
}

// ── NATS consumer task ────────────────────────────────────────────────────────

/// Spawn a background task that subscribes `subject` on `bus` and drives
/// `consumer.process_message` for every incoming message.
///
/// Errors from individual messages are logged but do not terminate the loop.
pub fn start_consumer(consumer: IngestConsumer, bus: Arc<dyn EventBus>, subject: impl Into<String>) {
    let subject = subject.into();
    tokio::spawn(async move {
        tracing::info!(consumer = %consumer.name, subject, "Starting ingestion consumer");

        let mut stream = match bus.subscribe(&subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    consumer = %consumer.name,
                    subject,
                    error = %e,
                    "Failed to subscribe"
                );
                return;
            }
        };

        while let Some(msg) = stream.next().await {
            match consumer.process_message(&msg).await {
                Ok(true) => {}
                Ok(false) => {}
                Err(e) => {
                    tracing::error!(
                        consumer = %consumer.name,
                        subject = %msg.subject,
                        error = %e,
                        "Failed to process message"
                    );
                }
            }
        }

        tracing::warn!(consumer = %consumer.name, subject, "Consumer stream ended");
    });
}

// ── Integrated tests (real DB + InMemoryBus, no external mocks) ───────────────

#[cfg(test)]
mod tests {
    use super::*;
    use event_bus::InMemoryBus;
    use serial_test::serial;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://ap_user:ap_pass@localhost:5443/reporting_test".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to reporting test DB");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Failed to run reporting migrations");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name LIKE 'test-ingest-%'",
        )
        .execute(pool)
        .await
        .ok();
    }

    /// A simple test handler that counts how many times it's been called.
    struct CountingHandler {
        call_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl StreamHandler for CountingHandler {
        async fn handle(
            &self,
            _pool: &PgPool,
            _tenant_id: &str,
            _event_id: &str,
            _payload: &serde_json::Value,
        ) -> Result<(), anyhow::Error> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn make_envelope(event_id: &str, tenant_id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": tenant_id,
            "data": {}
        }))
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_process_message_calls_handler_once() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let call_count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler { call_count: call_count.clone() });
        let consumer = IngestConsumer::new("test-ingest-basic", pool.clone(), handler);

        let msg = BusMessage::new(
            "test.subject".to_string(),
            make_envelope("evt-001", "tenant-x"),
        );

        let processed = consumer.process_message(&msg).await.expect("process failed");
        assert!(processed, "first call should process the event");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_process_message_skips_duplicate_event_id() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let call_count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler { call_count: call_count.clone() });
        let consumer = IngestConsumer::new("test-ingest-dedup", pool.clone(), handler);

        let msg = BusMessage::new(
            "test.subject".to_string(),
            make_envelope("evt-dedup-001", "tenant-y"),
        );

        // First delivery
        consumer.process_message(&msg).await.expect("first process failed");
        // Second delivery (same event_id)
        let processed = consumer.process_message(&msg).await.expect("second process failed");

        assert!(!processed, "duplicate event must be skipped");
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "handler must be called exactly once despite duplicate delivery"
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_process_message_handles_different_tenants_independently() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let call_count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler { call_count: call_count.clone() });
        let consumer = IngestConsumer::new("test-ingest-tenants", pool.clone(), handler);

        // Same event_id, different tenants → both should be processed
        let msg_a = BusMessage::new(
            "test.subject".to_string(),
            make_envelope("evt-same-id", "tenant-a"),
        );
        let msg_b = BusMessage::new(
            "test.subject".to_string(),
            make_envelope("evt-same-id", "tenant-b"),
        );

        consumer.process_message(&msg_a).await.expect("msg_a failed");
        consumer.process_message(&msg_b).await.expect("msg_b failed");

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            2,
            "same event_id for different tenants must both be processed"
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_start_consumer_delivers_message_via_bus() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let call_count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler { call_count: call_count.clone() });
        let consumer = IngestConsumer::new("test-ingest-bus", pool.clone(), handler);

        let bus = Arc::new(InMemoryBus::new());
        let subject = "test.ingest.bus.events";

        start_consumer(consumer, bus.clone(), subject);

        // Give the background task a moment to subscribe
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let payload = make_envelope("evt-bus-001", "tenant-bus");
        bus.publish(subject, payload).await.expect("publish failed");

        // Allow processing to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "handler must be called once for the published message"
        );

        cleanup(&pool).await;
    }
}

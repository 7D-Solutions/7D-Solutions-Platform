//! JetStream consumer manager: pull-based consumption with dedupe,
//! retry, and DLQ routing.
//!
//! Provides [`JetStreamConsumer`] which pulls messages from a NATS
//! JetStream stream, decodes [`EventEnvelope`]s, routes them through
//! an [`EventRouter`], applies idempotency via [`with_dedupe`], retries
//! transient failures, and dead-letters terminal failures.
//!
//! Acks only after the message is fully resolved (success or DLQ).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_nats::jetstream::{self, consumer::pull};
use event_bus::consumer_retry::RetryConfig;
use event_bus::EventEnvelope;
use futures::StreamExt;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use crate::dlq::{classify_handler_error, write_dlq_entry, FailureKind};
use crate::idempotency::{with_dedupe, DedupeError, DedupeOutcome};
use crate::router::{EventRouter, RouteOutcome};

/// Configuration for a JetStream consumer.
#[derive(Debug, Clone)]
pub struct ConsumerConfig {
    /// JetStream stream name (e.g. "PRODUCTION_EVENTS").
    pub stream_name: String,
    /// Durable consumer name (e.g. "inventory-consumer").
    pub consumer_name: String,
    /// Subject filter (e.g. "production.events.>").
    pub filter_subject: String,
    /// Retry policy for transient handler failures.
    pub retry_config: RetryConfig,
}

/// Error from consumer setup or operation.
#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error("setup: {0}")]
    Setup(String),
}

/// Point-in-time snapshot of consumer health metrics.
#[derive(Debug, Clone)]
pub struct HealthSnapshot {
    pub messages_processed: u64,
    pub messages_skipped: u64,
    pub messages_dlq: u64,
    pub is_running: bool,
}

impl HealthSnapshot {
    pub fn is_healthy(&self) -> bool {
        self.is_running
    }
}

/// Thread-safe health counters for consumer monitoring.
///
/// Obtain via [`JetStreamConsumer::health`], then call
/// [`snapshot`](ConsumerHealth::snapshot) from your HTTP health endpoint.
pub struct ConsumerHealth {
    processed: AtomicU64,
    skipped: AtomicU64,
    dlq: AtomicU64,
    running: AtomicBool,
}

impl ConsumerHealth {
    fn new() -> Self {
        Self {
            processed: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            dlq: AtomicU64::new(0),
            running: AtomicBool::new(false),
        }
    }

    pub fn snapshot(&self) -> HealthSnapshot {
        HealthSnapshot {
            messages_processed: self.processed.load(Ordering::Relaxed),
            messages_skipped: self.skipped.load(Ordering::Relaxed),
            messages_dlq: self.dlq.load(Ordering::Relaxed),
            is_running: self.running.load(Ordering::Relaxed),
        }
    }
}

/// JetStream consumer manager.
///
/// Pulls messages, decodes envelopes, routes through handlers with
/// idempotency + retry, dead-letters failures, acks only after resolution.
pub struct JetStreamConsumer {
    config: ConsumerConfig,
    router: Arc<EventRouter>,
    pool: PgPool,
    js: jetstream::Context,
    health: Arc<ConsumerHealth>,
}

impl JetStreamConsumer {
    pub fn new(
        nats_client: async_nats::Client,
        config: ConsumerConfig,
        router: EventRouter,
        pool: PgPool,
    ) -> Self {
        let js = jetstream::new(nats_client);
        Self {
            config,
            router: Arc::new(router),
            pool,
            js,
            health: Arc::new(ConsumerHealth::new()),
        }
    }

    pub fn health(&self) -> Arc<ConsumerHealth> {
        Arc::clone(&self.health)
    }

    /// Run the consumption loop until `shutdown` signals `true`.
    pub async fn run(
        &self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ConsumerError> {
        let stream = self
            .js
            .get_stream(&self.config.stream_name)
            .await
            .map_err(|e| {
                ConsumerError::Setup(format!("stream '{}': {}", self.config.stream_name, e))
            })?;

        let consumer = stream
            .get_or_create_consumer(
                &self.config.consumer_name,
                pull::Config {
                    durable_name: Some(self.config.consumer_name.clone()),
                    filter_subject: self.config.filter_subject.clone(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| {
                ConsumerError::Setup(format!("consumer '{}': {}", self.config.consumer_name, e))
            })?;

        let mut messages = consumer
            .messages()
            .await
            .map_err(|e| ConsumerError::Setup(format!("messages: {e}")))?;

        self.health.running.store(true, Ordering::Relaxed);
        info!(
            stream = %self.config.stream_name,
            consumer = %self.config.consumer_name,
            "JetStream consumer started"
        );

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Shutdown signal received");
                        break;
                    }
                }
                msg = messages.next() => {
                    match msg {
                        Some(Ok(message)) => self.process_message(message).await,
                        Some(Err(e)) => {
                            warn!(error = %e, "JetStream receive error");
                        }
                        None => {
                            info!("Message stream ended");
                            break;
                        }
                    }
                }
            }
        }

        self.health.running.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn process_message(&self, message: jetstream::Message) {
        let subject = message.subject.to_string();

        let envelope: EventEnvelope<serde_json::Value> =
            match serde_json::from_slice(&message.payload) {
                Ok(e) => e,
                Err(e) => {
                    warn!(subject = %subject, error = %e, "Poison: decode failed");
                    let _ = write_dlq_entry(
                        &self.pool,
                        uuid::Uuid::new_v4(),
                        &subject,
                        FailureKind::Poison,
                        &format!("decode: {e}"),
                        &serde_json::json!({"raw": String::from_utf8_lossy(&message.payload)}),
                    )
                    .await;
                    self.health.dlq.fetch_add(1, Ordering::Relaxed);
                    let _ = message.ack().await;
                    return;
                }
            };

        let event_id = envelope.event_id;
        let max = self.config.retry_config.max_attempts;
        let mut backoff = self.config.retry_config.initial_backoff;
        let max_backoff = self.config.retry_config.max_backoff;
        let mut last_err: Option<String> = None;

        for attempt in 1..=max {
            let router = Arc::clone(&self.router);
            let pool = self.pool.clone();
            let subj = subject.clone();
            let env = envelope.clone();

            let result = with_dedupe(&self.pool, event_id, &subject, || async move {
                Self::handle_routed(router, pool, event_id, subj, env).await
            })
            .await;

            match result {
                Ok(DedupeOutcome::Executed) => {
                    self.health.processed.fetch_add(1, Ordering::Relaxed);
                    let _ = message.ack().await;
                    return;
                }
                Ok(DedupeOutcome::Duplicate) => {
                    self.health.skipped.fetch_add(1, Ordering::Relaxed);
                    let _ = message.ack().await;
                    return;
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                    if attempt < max {
                        debug!(
                            event_id = %event_id,
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            error = %e,
                            "Retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(max_backoff);
                    }
                }
            }
        }

        // Retries exhausted → DLQ
        let err_msg = last_err.unwrap_or_else(|| "unknown".into());
        warn!(event_id = %event_id, attempts = max, "Retries exhausted, DLQ");
        let _ = write_dlq_entry(
            &self.pool,
            event_id,
            &subject,
            FailureKind::Retryable,
            &err_msg,
            &serde_json::to_value(&envelope).unwrap_or_default(),
        )
        .await;
        self.health.dlq.fetch_add(1, Ordering::Relaxed);
        let _ = message.ack().await;
    }

    /// Route an envelope and map the outcome to dedupe-compatible Result.
    async fn handle_routed(
        router: Arc<EventRouter>,
        pool: PgPool,
        event_id: uuid::Uuid,
        subject: String,
        envelope: EventEnvelope<serde_json::Value>,
    ) -> Result<(), DedupeError> {
        let outcome = router.route(&subject, &envelope).await;
        match outcome {
            RouteOutcome::Handled | RouteOutcome::Skipped => Ok(()),
            RouteOutcome::Invalid(reason) => {
                let _ = write_dlq_entry(
                    &pool,
                    event_id,
                    &subject,
                    FailureKind::Poison,
                    &reason,
                    &serde_json::to_value(&envelope).unwrap_or_default(),
                )
                .await;
                Ok(())
            }
            RouteOutcome::DeadLettered(reason) => {
                let _ = write_dlq_entry(
                    &pool,
                    event_id,
                    &subject,
                    FailureKind::Fatal,
                    &reason,
                    &serde_json::to_value(&envelope).unwrap_or_default(),
                )
                .await;
                Ok(())
            }
            RouteOutcome::HandlerError(handler_err) => {
                let kind = classify_handler_error(&handler_err);
                if kind == FailureKind::Retryable {
                    Err(DedupeError::Handler(handler_err.to_string()))
                } else {
                    let _ = write_dlq_entry(
                        &pool,
                        event_id,
                        &subject,
                        kind,
                        &handler_err.to_string(),
                        &serde_json::to_value(&envelope).unwrap_or_default(),
                    )
                    .await;
                    Ok(())
                }
            }
        }
    }
}

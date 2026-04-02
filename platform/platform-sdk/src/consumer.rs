//! Consumer wiring with retry for the platform SDK.
//!
//! Registers event consumers in the builder, subscribes to event-bus
//! subjects, and wraps each handler with exponential-backoff retry
//! (3 attempts, 100 ms → 30 s — matching event-bus defaults).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{EventBus, EventEnvelope, TracingContext};
use futures::StreamExt;
use tracing::Instrument;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use uuid::Uuid;

use crate::context::ModuleContext;
use crate::startup::StartupError;

/// NATS subject for the tenant.provisioned event.
///
/// Matches `tenant_registry::event_types::TENANT_PROVISIONED` without
/// coupling the SDK to the control-plane's tenant-registry crate.
pub(crate) const TENANT_PROVISIONED_SUBJECT: &str = "tenant.provisioned";

/// Error returned by a consumer handler.
#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error("{0}")]
    Processing(String),
}

/// Type-erased async consumer handler.
pub(crate) type BoxedHandler = Arc<
    dyn Fn(
            ModuleContext,
            EventEnvelope<serde_json::Value>,
        ) -> Pin<Box<dyn Future<Output = Result<(), ConsumerError>> + Send>>
        + Send
        + Sync,
>;

/// Event payload received by the [`ModuleBuilder::on_tenant_provisioned`] hook.
///
/// Contains the tenant ID whose provisioning just completed. The module's
/// database has been created and migrated by the orchestrator — use this
/// hook for module-specific setup (seed data, default config).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TenantProvisionedEvent {
    /// The newly provisioned tenant.
    pub tenant_id: Uuid,
}

/// Type-erased async handler for the provisioning hook.
pub type ProvisioningHandler = Arc<
    dyn Fn(
            ModuleContext,
            TenantProvisionedEvent,
        ) -> Pin<Box<dyn Future<Output = Result<(), ConsumerError>> + Send>>
        + Send
        + Sync,
>;

/// A consumer registration: subject + handler.
pub struct ConsumerDef {
    pub(crate) subject: String,
    pub(crate) handler: BoxedHandler,
}

impl ConsumerDef {
    /// Create a consumer definition from a subject and an async handler.
    pub fn new<F, Fut>(subject: impl Into<String>, handler: F) -> Self
    where
        F: Fn(ModuleContext, EventEnvelope<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ConsumerError>> + Send + 'static,
    {
        Self {
            subject: subject.into(),
            handler: Arc::new(move |ctx, env| Box::pin(handler(ctx, env))),
        }
    }
}

/// Handles for running consumer tasks — used during shutdown to drain.
pub struct ConsumerHandles {
    shutdown_tx: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl ConsumerHandles {
    pub(crate) fn empty() -> Self {
        let (tx, _) = watch::channel(false);
        Self {
            shutdown_tx: tx,
            tasks: vec![],
        }
    }

    /// Add a task handle to be drained on shutdown.
    pub(crate) fn add_task(&mut self, handle: JoinHandle<()>) {
        self.tasks.push(handle);
    }

    /// Get a shutdown receiver for sharing with additional tasks.
    pub(crate) fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    /// Signal all consumers to stop, then wait for in-flight work to finish.
    pub async fn shutdown(self) {
        if self.tasks.is_empty() {
            return;
        }
        let _ = self.shutdown_tx.send(true);
        for handle in self.tasks {
            let _ = handle.await;
        }
    }
}

/// NATS subject prefix for dead-letter messages.
///
/// Malformed events that cannot be deserialized are published here so
/// operators can inspect them without losing the raw payload.
/// Subject pattern: `dlq.{original_subject}`.
const DLQ_PREFIX: &str = "dlq";

/// Publish a malformed event to the dead-letter queue.
///
/// The DLQ payload is a JSON object with:
/// - `original_subject` — where the event arrived
/// - `error` — the deserialization error message
/// - `failed_at` — RFC 3339 timestamp
/// - `raw_payload` — original bytes as a JSON byte array
///
/// If the DLQ publish itself fails, logs a warning and continues.
async fn publish_to_dlq(bus: &Arc<dyn EventBus>, subject: &str, raw: &[u8], error: &str) {
    let dlq_subject = format!("{DLQ_PREFIX}.{subject}");
    let envelope = serde_json::json!({
        "original_subject": subject,
        "error": error,
        "failed_at": chrono::Utc::now().to_rfc3339(),
        "raw_payload": raw,
    });
    match serde_json::to_vec(&envelope) {
        Ok(bytes) => {
            if let Err(e) = bus.publish(&dlq_subject, bytes).await {
                tracing::warn!(
                    subject = %subject,
                    dlq = %dlq_subject,
                    error = %e,
                    "failed to publish malformed event to DLQ"
                );
            } else {
                tracing::warn!(
                    subject = %subject,
                    dlq = %dlq_subject,
                    "malformed event sent to DLQ"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                subject = %subject,
                error = %e,
                "failed to serialize DLQ envelope — raw payload lost"
            );
        }
    }
}

/// Subscribe and spawn a task for each consumer definition.
pub async fn wire_consumers(
    consumers: Vec<ConsumerDef>,
    bus: &Arc<dyn EventBus>,
    ctx: &ModuleContext,
) -> Result<ConsumerHandles, StartupError> {
    if consumers.is_empty() {
        return Ok(ConsumerHandles::empty());
    }

    // Warn on duplicate subjects — silently splitting messages between
    // handlers is a common source of bugs.
    {
        let mut seen = std::collections::HashSet::new();
        for def in &consumers {
            if !seen.insert(&def.subject) {
                tracing::warn!(
                    subject = %def.subject,
                    "duplicate consumer subject — messages will be split between handlers"
                );
            }
        }
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut tasks = Vec::with_capacity(consumers.len());
    let retry_config = RetryConfig::default();

    for def in consumers {
        tracing::info!(subject = %def.subject, "subscribing consumer");

        let mut stream = bus.subscribe(&def.subject).await.map_err(|e| {
            StartupError::Config(format!("failed to subscribe to '{}': {e}", def.subject))
        })?;

        let handler = def.handler;
        let ctx = ctx.clone();
        let subject = def.subject;
        let mut rx = shutdown_rx.clone();
        let retry_cfg = retry_config.clone();
        let bus_clone = Arc::clone(bus);

        let handle = tokio::spawn(async move {
            let context_label = format!("consumer:{subject}");
            tracing::info!(subject = %subject, "consumer listening");

            loop {
                tokio::select! {
                    biased;
                    _ = rx.changed() => {
                        if *rx.borrow() {
                            break;
                        }
                    }
                    msg = stream.next() => {
                        let Some(msg) = msg else { break; };

                        let envelope: EventEnvelope<serde_json::Value> =
                            match serde_json::from_slice(&msg.payload) {
                                Ok(e) => e,
                                Err(e) => {
                                    tracing::error!(
                                        subject = %subject, error = %e,
                                        "envelope deserialization failed — routing to DLQ"
                                    );
                                    publish_to_dlq(&bus_clone, &subject, &msg.payload, &e.to_string()).await;
                                    continue;
                                }
                            };

                        let event_id = envelope.event_id;
                        let tracing_ctx = TracingContext::from_envelope(&envelope);

                        let span = tracing::info_span!(
                            "event_consumer",
                            subject = %subject,
                            event_id = %event_id,
                            correlation_id = tracing_ctx.correlation_id.as_deref().unwrap_or(""),
                            trace_id = tracing_ctx.trace_id.as_deref().unwrap_or(""),
                            causation_id = tracing_ctx.causation_id.as_deref().unwrap_or(""),
                        );

                        let result = retry_with_backoff(
                            || {
                                let h = handler.clone();
                                let c = ctx.clone();
                                let env = envelope.clone();
                                async move { h(c, env).await.map_err(|e| e.to_string()) }
                            },
                            &retry_cfg,
                            &context_label,
                        )
                        .instrument(span)
                        .await;

                        if let Err(e) = result {
                            tracing::error!(
                                subject = %subject, event_id = %event_id,
                                error = %e, "consumer exhausted retries"
                            );
                        }
                    }
                }
            }

            tracing::info!(subject = %subject, "consumer stopped");
        });

        tasks.push(handle);
    }

    Ok(ConsumerHandles { shutdown_tx, tasks })
}

/// Subscribe to `tenant.provisioned` via the event bus and run the hook.
///
/// The provisioning outbox relay publishes raw JSON payloads (not
/// EventEnvelope), so this function deserializes directly into
/// [`TenantProvisionedEvent`].
pub async fn wire_provisioning_hook(
    handler: ProvisioningHandler,
    bus: &Arc<dyn EventBus>,
    ctx: &ModuleContext,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<JoinHandle<()>, StartupError> {
    let mut stream = bus.subscribe(TENANT_PROVISIONED_SUBJECT).await.map_err(|e| {
        StartupError::Config(format!(
            "failed to subscribe to '{}': {e}",
            TENANT_PROVISIONED_SUBJECT
        ))
    })?;

    tracing::info!(
        subject = TENANT_PROVISIONED_SUBJECT,
        "provisioning hook subscribed"
    );

    let ctx = ctx.clone();
    let mut rx = shutdown_rx;
    let retry_config = RetryConfig::default();
    let bus_clone = Arc::clone(bus);

    let handle = tokio::spawn(async move {
        tracing::info!(subject = TENANT_PROVISIONED_SUBJECT, "provisioning hook listening");

        loop {
            tokio::select! {
                biased;
                _ = rx.changed() => {
                    if *rx.borrow() {
                        break;
                    }
                }
                msg = stream.next() => {
                    let Some(msg) = msg else { break; };

                    let event: TenantProvisionedEvent =
                        match serde_json::from_slice(&msg.payload) {
                            Ok(e) => e,
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "provisioning hook: failed to parse event — routing to DLQ"
                                );
                                publish_to_dlq(
                                    &bus_clone,
                                    TENANT_PROVISIONED_SUBJECT,
                                    &msg.payload,
                                    &e.to_string(),
                                )
                                .await;
                                continue;
                            }
                        };

                    let tenant_id = event.tenant_id;
                    let span = tracing::info_span!(
                        "provisioning_hook",
                        tenant_id = %tenant_id,
                    );

                    let result = retry_with_backoff(
                        || {
                            let h = handler.clone();
                            let c = ctx.clone();
                            let ev = event.clone();
                            async move { h(c, ev).await.map_err(|e| e.to_string()) }
                        },
                        &retry_config,
                        "provisioning_hook",
                    )
                    .instrument(span)
                    .await;

                    if let Err(e) = result {
                        tracing::error!(
                            tenant_id = %tenant_id,
                            error = %e,
                            "provisioning hook exhausted retries"
                        );
                    }
                }
            }
        }

        tracing::info!("provisioning hook stopped");
    });

    Ok(handle)
}

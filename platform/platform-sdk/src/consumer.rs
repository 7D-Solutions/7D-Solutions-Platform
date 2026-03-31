//! Consumer wiring with retry for the platform SDK.
//!
//! Registers event consumers in the builder, subscribes to event-bus
//! subjects, and wraps each handler with exponential-backoff retry
//! (3 attempts, 100 ms → 30 s — matching event-bus defaults).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{EventBus, EventEnvelope};
use futures::StreamExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::context::ModuleContext;
use crate::startup::StartupError;

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

/// Subscribe and spawn a task for each consumer definition.
pub async fn wire_consumers(
    consumers: Vec<ConsumerDef>,
    bus: &Arc<dyn EventBus>,
    ctx: &ModuleContext,
) -> Result<ConsumerHandles, StartupError> {
    if consumers.is_empty() {
        return Ok(ConsumerHandles::empty());
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
                                        "envelope deserialization failed — skipping"
                                    );
                                    continue;
                                }
                            };

                        let event_id = envelope.event_id;

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

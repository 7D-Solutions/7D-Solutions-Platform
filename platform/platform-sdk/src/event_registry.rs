//! Typed event dispatch registry for the platform SDK.
//!
//! Maps `(event_type, schema_version)` pairs to strongly-typed handler
//! functions. Unrecognized pairs are logged and skipped — never panicked.
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_sdk::event_registry::EventRegistry;
//!
//! let registry = EventRegistry::new()
//!     .on::<InvoiceOpened>("invoice.opened", "1.0.0", |ctx, env| async move {
//!         // env.payload is InvoiceOpened, already deserialized
//!         Ok(())
//!     })
//!     .on::<InvoiceClosed>("invoice.closed", "1.0.0", |ctx, env| async move {
//!         Ok(())
//!     });
//!
//! ModuleBuilder::from_manifest("module.toml")
//!     .event_registry("ar.events", registry)
//!     .run()
//!     .await?;
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use event_bus::EventEnvelope;
use serde::de::DeserializeOwned;

use crate::consumer::ConsumerError;
use crate::context::ModuleContext;

type RegistryHandler = Arc<
    dyn Fn(
            ModuleContext,
            EventEnvelope<serde_json::Value>,
        ) -> Pin<Box<dyn Future<Output = Result<(), ConsumerError>> + Send>>
        + Send
        + Sync,
>;

/// Typed event dispatch registry.
///
/// Maps `(event_type, schema_version)` pairs to strongly-typed async handler
/// functions. Handlers receive a fully-typed `EventEnvelope<T>` — payload
/// deserialization is handled internally.
///
/// Unrecognized `(event_type, schema_version)` pairs emit a `WARN` log entry
/// and return `Ok(())` — they are never an error.
///
/// Register this with [`ModuleBuilder::event_registry`] to subscribe it to a
/// NATS subject.
#[derive(Clone)]
pub struct EventRegistry {
    handlers: HashMap<(String, String), RegistryHandler>,
}

impl EventRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a typed handler for `(event_type, schema_version)`.
    ///
    /// If a handler is already registered for this key, it is replaced.
    /// The handler receives a [`ModuleContext`] and an `EventEnvelope<T>`
    /// with the payload already deserialized from the raw JSON.
    ///
    /// # Type Parameters
    ///
    /// * `T` — payload type; must implement `serde::de::DeserializeOwned`
    pub fn on<T, F, Fut>(
        mut self,
        event_type: impl Into<String>,
        schema_version: impl Into<String>,
        handler: F,
    ) -> Self
    where
        T: DeserializeOwned + Send + 'static,
        F: Fn(ModuleContext, EventEnvelope<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ConsumerError>> + Send + 'static,
    {
        let key = (event_type.into(), schema_version.into());
        let handler = Arc::new(handler);

        let wrapped: RegistryHandler = Arc::new(move |ctx, env| {
            let handler = Arc::clone(&handler);
            Box::pin(async move {
                let EventEnvelope {
                    event_id,
                    event_type,
                    occurred_at,
                    tenant_id,
                    source_module,
                    source_version,
                    schema_version,
                    trace_id,
                    correlation_id,
                    causation_id,
                    reverses_event_id,
                    supersedes_event_id,
                    side_effect_id,
                    replay_safe,
                    mutation_class,
                    actor_id,
                    actor_type,
                    merchant_context,
                    payload: raw_payload,
                } = env;

                let payload: T = serde_json::from_value(raw_payload).map_err(|e| {
                    ConsumerError::Processing(format!(
                        "event_registry: payload deserialization failed \
                         for {event_type}/{schema_version}: {e}"
                    ))
                })?;

                let typed_env = EventEnvelope {
                    event_id,
                    event_type,
                    occurred_at,
                    tenant_id,
                    source_module,
                    source_version,
                    schema_version,
                    trace_id,
                    correlation_id,
                    causation_id,
                    reverses_event_id,
                    supersedes_event_id,
                    side_effect_id,
                    replay_safe,
                    mutation_class,
                    actor_id,
                    actor_type,
                    merchant_context,
                    payload,
                };

                handler(ctx, typed_env).await
            })
        });

        self.handlers.insert(key, wrapped);
        self
    }

    /// Dispatch an incoming event to its registered handler.
    ///
    /// Looks up the handler by `(event_type, schema_version)`. If no handler
    /// is registered, logs a warning and returns `Ok(())` — unrecognized
    /// events are never an error.
    pub async fn dispatch(
        &self,
        ctx: ModuleContext,
        env: EventEnvelope<serde_json::Value>,
    ) -> Result<(), ConsumerError> {
        let key = (env.event_type.clone(), env.schema_version.clone());
        match self.handlers.get(&key) {
            Some(handler) => handler(ctx, env).await,
            None => {
                tracing::warn!(
                    event_type = %env.event_type,
                    schema_version = %env.schema_version,
                    event_id = %env.event_id,
                    "event_registry: no handler registered for this event type/version — skipping"
                );
                Ok(())
            }
        }
    }

    /// Dispatch an event with atomic idempotency deduplication.
    ///
    /// Opens a database transaction and calls
    /// [`idempotency::check_and_mark`][crate::idempotency::check_and_mark]
    /// within it.  If `env.event_id` is already in `dedupe_table` the event
    /// is skipped silently and `Ok(())` is returned.
    ///
    /// If the event is new, the handler runs via
    /// [`dispatch`][Self::dispatch].  On handler success the dedup INSERT is
    /// committed; on handler failure the INSERT is rolled back so the event
    /// can be retried without being permanently blocked by a phantom dedup
    /// entry.
    ///
    /// # Atomicity note
    ///
    /// The dedup INSERT and the handler's own DB work are in separate
    /// connections.  The guarantee is: if the handler fails its own work, the
    /// dedup INSERT is also rolled back, allowing a retry.  If you need the
    /// handler's mutations and the dedup entry to commit in a single Postgres
    /// transaction, pass a shared `sqlx::Transaction` directly to your handler
    /// instead.
    pub async fn dispatch_with_dedup(
        &self,
        ctx: ModuleContext,
        env: EventEnvelope<serde_json::Value>,
        dedupe_table: &str,
    ) -> Result<(), ConsumerError> {
        let event_id = env.event_id;

        let mut tx = ctx
            .pool()
            .begin()
            .await
            .map_err(|e| {
                ConsumerError::Processing(format!(
                    "dedup: failed to begin transaction: {e}"
                ))
            })?;

        let is_new =
            crate::idempotency::check_and_mark(&mut tx, dedupe_table, event_id)
                .await
                .map_err(|e| {
                    ConsumerError::Processing(format!(
                        "dedup: check_and_mark failed for event {event_id}: {e}"
                    ))
                })?;

        if !is_new {
            tracing::debug!(
                event_id = %event_id,
                event_type = %env.event_type,
                dedupe_table = %dedupe_table,
                "event_registry: duplicate event_id — skipping"
            );
            // INSERT was a no-op; rollback is equivalent to commit here.
            let _ = tx.rollback().await;
            return Ok(());
        }

        match self.dispatch(ctx, env).await {
            Ok(()) => {
                tx.commit().await.map_err(|e| {
                    ConsumerError::Processing(format!(
                        "dedup: commit failed for event {event_id}: {e}"
                    ))
                })?;
                Ok(())
            }
            Err(e) => {
                // Roll back the dedup INSERT — the event can be retried.
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Dispatch an event and write it to the DLQ on handler failure.
    ///
    /// Behaves identically to [`dispatch`][Self::dispatch] on success.
    /// When the handler returns an error the event is written to `dlq_table`
    /// using a **fresh connection** from `ctx.pool()` — this write is always
    /// in a separate transaction from the handler's own work, so it succeeds
    /// even if the handler's transaction was rolled back.
    ///
    /// After the DLQ write this method returns `Ok(())` so the NATS message
    /// is acknowledged and not redelivered. The entry remains in the DLQ for
    /// manual inspection and replay via [`crate::dlq::replay_dlq_entry`].
    ///
    /// If the DLQ write itself fails, the error is logged but `Ok(())` is
    /// still returned — the original handler error is never surfaced to the
    /// caller.
    pub async fn dispatch_with_dlq(
        &self,
        ctx: ModuleContext,
        env: EventEnvelope<serde_json::Value>,
        dlq_table: &str,
    ) -> Result<(), ConsumerError> {
        // Snapshot envelope fields needed for DLQ before moving `env`.
        let event_id = env.event_id;
        let event_type = env.event_type.clone();
        let schema_version = env.schema_version.clone();
        let tenant_id = env.tenant_id.clone();
        let payload = env.payload.clone();

        match self.dispatch(ctx.clone(), env).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let error_message = e.to_string();
                tracing::warn!(
                    event_id = %event_id,
                    event_type = %event_type,
                    error = %error_message,
                    dlq_table = %dlq_table,
                    "event_registry: handler failed — writing to DLQ"
                );
                if let Err(dlq_err) = crate::dlq::write_dlq_entry(
                    ctx.pool(),
                    dlq_table,
                    event_id,
                    &event_type,
                    &schema_version,
                    &tenant_id,
                    &payload,
                    &error_message,
                    0,
                )
                .await
                {
                    tracing::error!(
                        event_id = %event_id,
                        event_type = %event_type,
                        dlq_err = %dlq_err,
                        "event_registry: DLQ write failed — event lost"
                    );
                }
                Ok(())
            }
        }
    }
}

impl Default for EventRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct OrderPlaced {
        order_id: String,
        amount: i64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct OrderCancelled {
        order_id: String,
        reason: String,
    }

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

    fn make_ctx() -> ModuleContext {
        let pool = sqlx::PgPool::connect_lazy("postgres://dummy:dummy@localhost/dummy")
            .expect("connect_lazy does not establish a connection");
        let manifest = crate::manifest::Manifest::from_str(
            "[module]\nname = \"test\"\nversion = \"0.1.0\"",
            None,
        )
        .expect("valid minimal manifest");
        ModuleContext::new(pool, manifest, None)
    }

    #[tokio::test]
    async fn dispatch_routes_to_correct_handler() {
        let called: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let called_placed = Arc::clone(&called);
        let called_cancelled = Arc::clone(&called);

        let registry = EventRegistry::new()
            .on(
                "order.placed",
                "1.0.0",
                move |_ctx, env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_placed);
                    async move {
                        called
                            .lock()
                            .expect("test assertion")
                            .push(format!("placed:{}", env.payload.order_id));
                        Ok(())
                    }
                },
            )
            .on(
                "order.cancelled",
                "1.0.0",
                move |_ctx, env: EventEnvelope<OrderCancelled>| {
                    let called = Arc::clone(&called_cancelled);
                    async move {
                        called
                            .lock()
                            .expect("test assertion")
                            .push(format!("cancelled:{}", env.payload.order_id));
                        Ok(())
                    }
                },
            );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "ord-1", "amount": 100}),
        );
        registry.dispatch(make_ctx(), env).await.expect("test assertion");

        let log = called.lock().expect("test assertion").clone();
        assert_eq!(log, vec!["placed:ord-1"]);
    }

    #[tokio::test]
    async fn dispatch_unknown_pair_returns_ok() {
        let registry = EventRegistry::new();
        let env = make_envelope("unknown.event", "9.9.9", serde_json::json!({}));
        assert!(registry.dispatch(make_ctx(), env).await.is_ok());
    }

    #[tokio::test]
    async fn dispatch_deserializes_payload_correctly() {
        let received: Arc<Mutex<Option<OrderPlaced>>> = Arc::new(Mutex::new(None));
        let received_clone = Arc::clone(&received);

        let registry = EventRegistry::new().on(
            "order.placed",
            "1.0.0",
            move |_ctx, env: EventEnvelope<OrderPlaced>| {
                let received = Arc::clone(&received_clone);
                async move {
                    *received.lock().expect("test assertion") = Some(env.payload.clone());
                    Ok(())
                }
            },
        );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "ord-42", "amount": 500}),
        );
        registry.dispatch(make_ctx(), env).await.expect("test assertion");

        let payload = received
            .lock()
            .expect("test assertion")
            .take()
            .expect("handler was called");
        assert_eq!(
            payload,
            OrderPlaced {
                order_id: "ord-42".into(),
                amount: 500
            }
        );
    }

    #[tokio::test]
    async fn on_replaces_duplicate_key() {
        let called: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

        let called_first = Arc::clone(&called);
        let called_second = Arc::clone(&called);

        let registry = EventRegistry::new()
            .on(
                "order.placed",
                "1.0.0",
                move |_ctx, _env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_first);
                    async move {
                        called.lock().expect("test assertion").push("first");
                        Ok(())
                    }
                },
            )
            .on(
                "order.placed",
                "1.0.0",
                move |_ctx, _env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_second);
                    async move {
                        called.lock().expect("test assertion").push("second");
                        Ok(())
                    }
                },
            );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "ord-1", "amount": 10}),
        );
        registry.dispatch(make_ctx(), env).await.expect("test assertion");

        let log = called.lock().expect("test assertion").clone();
        assert_eq!(log, vec!["second"], "second registration should replace first");
    }

    #[tokio::test]
    async fn dispatch_returns_error_on_bad_payload() {
        let registry = EventRegistry::new().on(
            "order.placed",
            "1.0.0",
            |_ctx, _env: EventEnvelope<OrderPlaced>| async { Ok(()) },
        );

        // Missing required fields — deserialization must fail
        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"unexpected_field": true}),
        );
        assert!(
            registry.dispatch(make_ctx(), env).await.is_err(),
            "bad payload should return a ConsumerError"
        );
    }
}

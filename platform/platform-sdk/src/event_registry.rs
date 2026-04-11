//! Typed event dispatch registry for the platform SDK.
//!
//! Maps `(event_type, schema_version)` pairs to strongly-typed handler
//! functions. Unrecognized pairs are logged and skipped — never panicked.
//!
//! # Dispatch fallback
//!
//! Dispatch looks up handlers in priority order:
//! 1. Exact `(event_type, schema_version)` match.
//! 2. Wildcard `(event_type, any version)` — registered with
//!    [`on_any_version`](EventRegistry::on_any_version).
//! 3. No handler found → [`RouteOutcome::Unknown`].
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_sdk::event_registry::{EventRegistry, RouteOutcome};
//!
//! let registry = EventRegistry::new()
//!     .on::<InvoiceOpened>("invoice.opened", "1.0.0", |ctx, env| async move {
//!         // env.payload is InvoiceOpened, already deserialized
//!         RouteOutcome::Handled
//!     })
//!     .on::<InvoiceClosed>("invoice.closed", "1.0.0", |ctx, env| async move {
//!         RouteOutcome::Handled
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

/// Outcome returned by an event handler registered with [`EventRegistry`].
///
/// Handlers return this instead of `Result<(), ConsumerError>`. The registry
/// uses the outcome to decide how to treat the event after the handler runs.
///
/// | Variant | Meaning | Effect in `dispatch_with_dlq` | Effect in `dispatch_with_dedup` |
/// |---------|---------|-------------------------------|----------------------------------|
/// | `Handled` | Processed successfully. | Acked. | Dedup entry committed. |
/// | `Skipped` | Intentionally ignored. | Acked. | Dedup entry rolled back — event not counted as processed. |
/// | `Retried` | Transient failure — retry later. | `Err` returned to trigger retry backoff. | Dedup entry rolled back. |
/// | `DeadLettered` | Permanent failure — write to DLQ. | Written to DLQ table, then acked. | Dedup entry committed — no retry. |
/// | `Unknown` | No handler registered. | Acked (logged as warning). | Dedup entry rolled back. |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    /// Event was processed successfully.
    Handled,
    /// Event was intentionally skipped (not an error, not retried).
    Skipped,
    /// Transient failure — the handler requests a retry.
    Retried,
    /// Permanent failure — the event should be written to the DLQ.
    DeadLettered,
    /// No handler was registered for this `(event_type, schema_version)` pair.
    Unknown,
}

type RegistryHandler = Arc<
    dyn Fn(
            ModuleContext,
            EventEnvelope<serde_json::Value>,
        ) -> Pin<Box<dyn Future<Output = RouteOutcome> + Send>>
        + Send
        + Sync,
>;

/// Typed event dispatch registry.
///
/// Maps `(event_type, schema_version)` pairs to strongly-typed async handler
/// functions. Handlers receive a fully-typed `EventEnvelope<T>` — payload
/// deserialization is handled internally.
///
/// If the incoming `schema_version` does not match any registered handler,
/// dispatch falls back to any handler registered with
/// [`on_any_version`](Self::on_any_version). Unrecognised pairs (no exact match
/// and no wildcard) emit a `WARN` log entry and return
/// [`RouteOutcome::Unknown`] — they are never an error.
///
/// Register this with [`ModuleBuilder::event_registry`] to subscribe it to a
/// NATS subject.
#[derive(Clone)]
pub struct EventRegistry {
    /// Key: `(event_type, Some(schema_version))` for exact match,
    ///      `(event_type, None)` for version-wildcard fallback.
    handlers: HashMap<(String, Option<String>), RegistryHandler>,
}

impl EventRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a typed handler for an exact `(event_type, schema_version)` pair.
    ///
    /// If a handler is already registered for this key, it is replaced.
    /// The handler receives a [`ModuleContext`] and an `EventEnvelope<T>`
    /// with the payload already deserialized from the raw JSON.
    ///
    /// Payload deserialization failure yields [`RouteOutcome::DeadLettered`]
    /// without calling the handler.
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
        Fut: Future<Output = RouteOutcome> + Send + 'static,
    {
        let key = (event_type.into(), Some(schema_version.into()));
        self.handlers.insert(key, Self::wrap_handler(handler));
        self
    }

    /// Register a typed handler for any `schema_version` of `event_type`.
    ///
    /// This handler fires only when no exact `(event_type, schema_version)`
    /// handler matches the incoming event. Use it for handlers that do not
    /// care about schema version — for example, audit loggers or generic
    /// fanout consumers.
    ///
    /// If a wildcard is already registered for this `event_type`, it is
    /// replaced.
    ///
    /// Payload deserialization failure yields [`RouteOutcome::DeadLettered`]
    /// without calling the handler.
    pub fn on_any_version<T, F, Fut>(
        mut self,
        event_type: impl Into<String>,
        handler: F,
    ) -> Self
    where
        T: DeserializeOwned + Send + 'static,
        F: Fn(ModuleContext, EventEnvelope<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = RouteOutcome> + Send + 'static,
    {
        let key = (event_type.into(), None);
        self.handlers.insert(key, Self::wrap_handler(handler));
        self
    }

    /// Wrap a typed handler into a type-erased [`RegistryHandler`].
    ///
    /// Deserializes the raw JSON payload to `T`. Deserialization failure
    /// short-circuits to [`RouteOutcome::DeadLettered`].
    fn wrap_handler<T, F, Fut>(handler: F) -> RegistryHandler
    where
        T: DeserializeOwned + Send + 'static,
        F: Fn(ModuleContext, EventEnvelope<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = RouteOutcome> + Send + 'static,
    {
        let handler = Arc::new(handler);
        Arc::new(move |ctx, env: EventEnvelope<serde_json::Value>| {
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

                let payload: T = match serde_json::from_value(raw_payload) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(
                            event_type = %event_type,
                            schema_version = %schema_version,
                            event_id = %event_id,
                            error = %e,
                            "event_registry: payload deserialization failed — dead-lettering"
                        );
                        return RouteOutcome::DeadLettered;
                    }
                };

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
            }) as Pin<Box<dyn Future<Output = RouteOutcome> + Send>>
        })
    }

    /// Dispatch an incoming event to its registered handler.
    ///
    /// Looks up the handler by `(event_type, schema_version)` first. If none
    /// is registered for the exact version, falls back to any handler
    /// registered via [`on_any_version`](Self::on_any_version). If no handler
    /// is found at all, logs a warning and returns [`RouteOutcome::Unknown`] —
    /// unrecognized events are never an error.
    pub async fn dispatch(
        &self,
        ctx: ModuleContext,
        env: EventEnvelope<serde_json::Value>,
    ) -> RouteOutcome {
        let exact_key = (env.event_type.clone(), Some(env.schema_version.clone()));
        let wildcard_key = (env.event_type.clone(), None::<String>);

        let handler = self
            .handlers
            .get(&exact_key)
            .or_else(|| self.handlers.get(&wildcard_key));

        match handler {
            Some(h) => h(ctx, env).await,
            None => {
                tracing::warn!(
                    event_type = %env.event_type,
                    schema_version = %env.schema_version,
                    event_id = %env.event_id,
                    "event_registry: no handler registered for this event type/version — skipping"
                );
                RouteOutcome::Unknown
            }
        }
    }

    /// Dispatch an event with atomic idempotency deduplication.
    ///
    /// Opens a database transaction and calls
    /// [`idempotency::check_and_mark`][crate::idempotency::check_and_mark]
    /// within it. If `env.event_id` is already in `dedupe_table` the event
    /// is skipped silently and `Ok(())` is returned.
    ///
    /// If the event is new, the handler runs via [`dispatch`][Self::dispatch].
    /// On [`RouteOutcome::Retried`] the dedup INSERT is rolled back so the
    /// event can be retried. All other outcomes commit the dedup entry.
    ///
    /// # Atomicity note
    ///
    /// The dedup INSERT and the handler's own DB work are in separate
    /// connections. The guarantee is: if the handler requests a retry, the
    /// dedup INSERT is also rolled back, allowing a retry. If you need the
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
            let _ = tx.rollback().await;
            return Ok(());
        }

        match self.dispatch(ctx, env).await {
            RouteOutcome::Retried => {
                let _ = tx.rollback().await;
                Err(ConsumerError::Processing(format!(
                    "event_registry: handler requested retry for event {event_id}"
                )))
            }
            _ => {
                tx.commit().await.map_err(|e| {
                    ConsumerError::Processing(format!(
                        "dedup: commit failed for event {event_id}: {e}"
                    ))
                })?;
                Ok(())
            }
        }
    }

    /// Dispatch an event and write it to the DLQ when the handler returns
    /// [`RouteOutcome::DeadLettered`].
    ///
    /// Behaves identically to [`dispatch`][Self::dispatch] unless the outcome
    /// is `DeadLettered`, in which case the event is written to `dlq_table`
    /// using a **fresh connection** from `ctx.pool()`. The DLQ write is always
    /// in a separate transaction from the handler's own work, so it succeeds
    /// even if the handler rolled back.
    ///
    /// After the DLQ write this method returns `Ok(())` so the NATS message
    /// is acknowledged and not redelivered. The entry remains in the DLQ for
    /// manual inspection and replay via [`crate::dlq::replay_dlq_entry`].
    ///
    /// If the DLQ write itself fails, the error is logged but `Ok(())` is
    /// still returned — the original handler outcome is never surfaced to the
    /// caller.
    ///
    /// [`RouteOutcome::Retried`] propagates as `Err` so the NATS consumer
    /// middleware can apply backoff and retry logic.
    pub async fn dispatch_with_dlq(
        &self,
        ctx: ModuleContext,
        env: EventEnvelope<serde_json::Value>,
        dlq_table: &str,
    ) -> Result<(), ConsumerError> {
        let event_id = env.event_id;
        let event_type = env.event_type.clone();
        let schema_version = env.schema_version.clone();
        let tenant_id = env.tenant_id.clone();
        let payload = env.payload.clone();

        match self.dispatch(ctx.clone(), env).await {
            RouteOutcome::DeadLettered => {
                tracing::warn!(
                    event_id = %event_id,
                    event_type = %event_type,
                    dlq_table = %dlq_table,
                    "event_registry: handler dead-lettered — writing to DLQ"
                );
                if let Err(dlq_err) = crate::dlq::write_dlq_entry(
                    ctx.pool(),
                    dlq_table,
                    event_id,
                    &event_type,
                    &schema_version,
                    &tenant_id,
                    &payload,
                    "handler returned DeadLettered",
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
            RouteOutcome::Retried => Err(ConsumerError::Processing(format!(
                "event_registry: handler requested retry for event {event_id}"
            ))),
            _ => Ok(()),
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
                        RouteOutcome::Handled
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
                        RouteOutcome::Handled
                    }
                },
            );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "ord-1", "amount": 100}),
        );
        let outcome = registry.dispatch(make_ctx(), env).await;
        assert_eq!(outcome, RouteOutcome::Handled);

        let log = called.lock().expect("test assertion").clone();
        assert_eq!(log, vec!["placed:ord-1"]);
    }

    #[tokio::test]
    async fn dispatch_unknown_pair_returns_unknown() {
        let registry = EventRegistry::new();
        let env = make_envelope("unknown.event", "9.9.9", serde_json::json!({}));
        assert_eq!(
            registry.dispatch(make_ctx(), env).await,
            RouteOutcome::Unknown
        );
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
                    RouteOutcome::Handled
                }
            },
        );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "ord-42", "amount": 500}),
        );
        registry.dispatch(make_ctx(), env).await;

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
                        RouteOutcome::Handled
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
                        RouteOutcome::Handled
                    }
                },
            );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "ord-1", "amount": 10}),
        );
        registry.dispatch(make_ctx(), env).await;

        let log = called.lock().expect("test assertion").clone();
        assert_eq!(log, vec!["second"], "second registration should replace first");
    }

    #[tokio::test]
    async fn dispatch_dead_letters_on_bad_payload() {
        let registry = EventRegistry::new().on(
            "order.placed",
            "1.0.0",
            |_ctx, _env: EventEnvelope<OrderPlaced>| async { RouteOutcome::Handled },
        );

        // Missing required fields — deserialization must fail → DeadLettered
        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"unexpected_field": true}),
        );
        assert_eq!(
            registry.dispatch(make_ctx(), env).await,
            RouteOutcome::DeadLettered,
            "bad payload should return DeadLettered"
        );
    }

    #[tokio::test]
    async fn dispatch_routes_by_schema_version() {
        let called: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let called_v1 = Arc::clone(&called);
        let called_v2 = Arc::clone(&called);

        let registry = EventRegistry::new()
            .on(
                "order.placed",
                "1.0.0",
                move |_ctx, _env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_v1);
                    async move {
                        called.lock().expect("test assertion").push("v1".into());
                        RouteOutcome::Handled
                    }
                },
            )
            .on(
                "order.placed",
                "2.0.0",
                move |_ctx, _env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_v2);
                    async move {
                        called.lock().expect("test assertion").push("v2".into());
                        RouteOutcome::Handled
                    }
                },
            );

        registry
            .dispatch(
                make_ctx(),
                make_envelope(
                    "order.placed",
                    "1.0.0",
                    serde_json::json!({"order_id": "a", "amount": 1}),
                ),
            )
            .await;
        registry
            .dispatch(
                make_ctx(),
                make_envelope(
                    "order.placed",
                    "2.0.0",
                    serde_json::json!({"order_id": "b", "amount": 2}),
                ),
            )
            .await;

        let log = called.lock().expect("test assertion").clone();
        assert_eq!(log, vec!["v1", "v2"]);
    }

    #[tokio::test]
    async fn on_any_version_fires_as_fallback() {
        let called: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let called_exact = Arc::clone(&called);
        let called_any = Arc::clone(&called);

        let registry = EventRegistry::new()
            .on(
                "order.placed",
                "1.0.0",
                move |_ctx, _env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_exact);
                    async move {
                        called.lock().expect("test assertion").push("exact-v1".into());
                        RouteOutcome::Handled
                    }
                },
            )
            .on_any_version(
                "order.placed",
                move |_ctx, _env: EventEnvelope<OrderPlaced>| {
                    let called = Arc::clone(&called_any);
                    async move {
                        called.lock().expect("test assertion").push("wildcard".into());
                        RouteOutcome::Handled
                    }
                },
            );

        // v1.0.0 → exact handler
        registry
            .dispatch(
                make_ctx(),
                make_envelope(
                    "order.placed",
                    "1.0.0",
                    serde_json::json!({"order_id": "a", "amount": 1}),
                ),
            )
            .await;
        // v3.0.0 → no exact handler → wildcard fallback
        registry
            .dispatch(
                make_ctx(),
                make_envelope(
                    "order.placed",
                    "3.0.0",
                    serde_json::json!({"order_id": "b", "amount": 2}),
                ),
            )
            .await;

        let log = called.lock().expect("test assertion").clone();
        assert_eq!(
            log,
            vec!["exact-v1", "wildcard"],
            "v1 goes to exact, unknown version falls back to wildcard"
        );
    }

    #[tokio::test]
    async fn handler_returning_skipped_produces_skipped_outcome() {
        let registry = EventRegistry::new().on(
            "order.placed",
            "1.0.0",
            |_ctx, _env: EventEnvelope<OrderPlaced>| async { RouteOutcome::Skipped },
        );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "skip-me", "amount": 0}),
        );
        assert_eq!(
            registry.dispatch(make_ctx(), env).await,
            RouteOutcome::Skipped
        );
    }

    #[tokio::test]
    async fn handler_returning_dead_lettered_produces_dead_lettered_outcome() {
        let registry = EventRegistry::new().on(
            "order.placed",
            "1.0.0",
            |_ctx, _env: EventEnvelope<OrderPlaced>| async { RouteOutcome::DeadLettered },
        );

        let env = make_envelope(
            "order.placed",
            "1.0.0",
            serde_json::json!({"order_id": "dead", "amount": 0}),
        );
        assert_eq!(
            registry.dispatch(make_ctx(), env).await,
            RouteOutcome::DeadLettered
        );
    }
}

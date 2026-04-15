use chrono::Utc;
use event_bus::EventEnvelope;
use tracing::{debug, warn};

use crate::context::HandlerContext;
use crate::registry::{HandlerError, HandlerRegistry, LookupResult};

/// Outcome of routing an event through the dispatcher.
#[derive(Debug)]
pub enum RouteOutcome {
    /// Handler ran successfully.
    Handled,
    /// Event type not registered — silently skipped (not for this consumer).
    Skipped,
    /// Known event type but unknown schema version — should be dead-lettered.
    DeadLettered(String),
    /// Envelope validation failed (e.g. empty tenant_id).
    Invalid(String),
    /// Handler returned an error.
    HandlerError(HandlerError),
}

impl RouteOutcome {
    /// Returns true if the event was successfully handled.
    pub fn is_handled(&self) -> bool {
        matches!(self, RouteOutcome::Handled)
    }

    /// Returns true if the event was skipped (unknown type).
    pub fn is_skipped(&self) -> bool {
        matches!(self, RouteOutcome::Skipped)
    }
}

/// Routes incoming events to registered handlers via the registry.
///
/// Validates envelope fields, looks up the handler, builds
/// [`HandlerContext`], and dispatches.
pub struct EventRouter {
    registry: HandlerRegistry,
}

impl EventRouter {
    /// Create a new router backed by the given handler registry.
    pub fn new(registry: HandlerRegistry) -> Self {
        Self { registry }
    }

    /// Route an event envelope to the appropriate handler.
    ///
    /// 1. Validates required envelope fields (tenant_id, event_type).
    /// 2. Looks up the handler by (event_type, schema_version).
    /// 3. Builds a [`HandlerContext`] from envelope metadata.
    /// 4. Dispatches to the handler and returns the outcome.
    pub async fn route(
        &self,
        subject: &str,
        envelope: &EventEnvelope<serde_json::Value>,
    ) -> RouteOutcome {
        if envelope.tenant_id.is_empty() {
            warn!(subject = %subject, event_id = %envelope.event_id, "Empty tenant_id");
            return RouteOutcome::Invalid("empty tenant_id".to_string());
        }

        if envelope.event_type.is_empty() {
            warn!(subject = %subject, event_id = %envelope.event_id, "Empty event_type");
            return RouteOutcome::Invalid("empty event_type".to_string());
        }

        match self
            .registry
            .lookup(&envelope.event_type, &envelope.schema_version)
        {
            LookupResult::UnknownType => {
                debug!(
                    subject = %subject,
                    event_type = %envelope.event_type,
                    "Unknown event type, skipping"
                );
                RouteOutcome::Skipped
            }
            LookupResult::UnknownVersion { known_versions } => {
                warn!(
                    subject = %subject,
                    event_type = %envelope.event_type,
                    schema_version = %envelope.schema_version,
                    ?known_versions,
                    "Unknown schema version, dead-lettering"
                );
                RouteOutcome::DeadLettered(format!(
                    "event_type={} schema_version={} not registered (known: {:?})",
                    envelope.event_type, envelope.schema_version, known_versions
                ))
            }
            LookupResult::Found(handler) => {
                let ctx = HandlerContext {
                    event_id: envelope.event_id,
                    tenant_id: envelope.tenant_id.clone(),
                    source_module: envelope.source_module.clone(),
                    correlation_id: envelope.correlation_id.clone(),
                    causation_id: envelope.causation_id.clone(),
                    actor_id: envelope.actor_id,
                    schema_version: envelope.schema_version.clone(),
                    received_at: Utc::now(),
                };

                match handler(ctx, envelope.payload.clone()).await {
                    Ok(()) => {
                        debug!(
                            subject = %subject,
                            event_type = %envelope.event_type,
                            event_id = %envelope.event_id,
                            "Handler completed successfully"
                        );
                        RouteOutcome::Handled
                    }
                    Err(e) => {
                        warn!(
                            subject = %subject,
                            event_type = %envelope.event_type,
                            event_id = %envelope.event_id,
                            error = %e,
                            "Handler returned error"
                        );
                        RouteOutcome::HandlerError(e)
                    }
                }
            }
        }
    }

    /// Access the underlying registry (e.g. for DLQ replay dispatch).
    pub fn registry(&self) -> &HandlerRegistry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{HandlerError, RegistryBuilder};
    use event_bus::EventEnvelope;
    use std::sync::{Arc, Mutex};

    fn make_envelope(event_type: &str, schema_version: &str) -> EventEnvelope<serde_json::Value> {
        EventEnvelope::new(
            "tenant-1".to_string(),
            "test-module".to_string(),
            event_type.to_string(),
            serde_json::json!({"key": "value"}),
        )
        .with_schema_version(schema_version.to_string())
    }

    fn make_envelope_empty_tenant(event_type: &str) -> EventEnvelope<serde_json::Value> {
        EventEnvelope::new(
            String::new(),
            "test-module".to_string(),
            event_type.to_string(),
            serde_json::json!({}),
        )
    }

    fn make_envelope_empty_type() -> EventEnvelope<serde_json::Value> {
        EventEnvelope::new(
            "tenant-1".to_string(),
            "test-module".to_string(),
            String::new(),
            serde_json::json!({}),
        )
    }

    #[tokio::test]
    async fn route_handled() {
        let called = Arc::new(Mutex::new(false));
        let called_clone = called.clone();

        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", move |_ctx, _payload| {
                let called = called_clone.clone();
                async move {
                    *called.lock().expect("mutex poisoned") = true;
                    Ok(())
                }
            })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope("test.event", "1.0.0");
        let outcome = router.route("test.subject", &envelope).await;

        assert!(outcome.is_handled());
        assert!(*called.lock().expect("mutex poisoned"));
    }

    #[tokio::test]
    async fn route_unknown_type_skipped() {
        let registry = RegistryBuilder::new()
            .register("known.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope("unknown.event", "1.0.0");
        let outcome = router.route("test.subject", &envelope).await;

        assert!(outcome.is_skipped());
    }

    #[tokio::test]
    async fn route_unknown_version_deadlettered() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope("test.event", "99.0.0");
        let outcome = router.route("test.subject", &envelope).await;

        assert!(matches!(outcome, RouteOutcome::DeadLettered(_)));
    }

    #[tokio::test]
    async fn route_empty_tenant_invalid() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope_empty_tenant("test.event");
        let outcome = router.route("test.subject", &envelope).await;

        assert!(matches!(outcome, RouteOutcome::Invalid(_)));
    }

    #[tokio::test]
    async fn route_empty_event_type_invalid() {
        let registry = RegistryBuilder::new().build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope_empty_type();
        let outcome = router.route("test.subject", &envelope).await;

        assert!(matches!(outcome, RouteOutcome::Invalid(_)));
    }

    #[tokio::test]
    async fn route_handler_error_surfaces() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async {
                Err(HandlerError::Transient("db timeout".to_string()))
            })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope("test.event", "1.0.0");
        let outcome = router.route("test.subject", &envelope).await;

        match outcome {
            RouteOutcome::HandlerError(HandlerError::Transient(msg)) => {
                assert_eq!(msg, "db timeout");
            }
            other => panic!("Expected HandlerError(Transient), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn route_passes_correct_context() {
        let captured_ctx = Arc::new(Mutex::new(None));
        let captured_clone = captured_ctx.clone();

        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", move |ctx, _payload| {
                let captured = captured_clone.clone();
                async move {
                    *captured.lock().expect("mutex poisoned") = Some(ctx);
                    Ok(())
                }
            })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope("test.event", "1.0.0");
        let outcome = router.route("test.subject", &envelope).await;

        assert!(outcome.is_handled());
        let ctx = captured_ctx
            .lock()
            .expect("mutex poisoned")
            .take()
            .expect("handler was not called");
        assert_eq!(ctx.tenant_id, "tenant-1");
        assert_eq!(ctx.source_module, "test-module");
        assert_eq!(ctx.event_id, envelope.event_id);
        assert_eq!(ctx.schema_version, "1.0.0");
    }

    #[tokio::test]
    async fn route_passes_payload() {
        let captured_payload = Arc::new(Mutex::new(None));
        let captured_clone = captured_payload.clone();

        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", move |_ctx, payload| {
                let captured = captured_clone.clone();
                async move {
                    *captured.lock().expect("mutex poisoned") = Some(payload);
                    Ok(())
                }
            })
            .build();

        let router = EventRouter::new(registry);
        let envelope = make_envelope("test.event", "1.0.0");
        let outcome = router.route("test.subject", &envelope).await;

        assert!(outcome.is_handled());
        let payload = captured_payload
            .lock()
            .expect("mutex poisoned")
            .take()
            .expect("handler was not called");
        assert_eq!(payload["key"], "value");
    }
}

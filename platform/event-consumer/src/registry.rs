use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::context::HandlerContext;

/// Error returned by event handlers.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Permanent failure — do not retry, send to DLQ.
    #[error("permanent: {0}")]
    Permanent(String),
    /// Transient failure — eligible for retry.
    #[error("transient: {0}")]
    Transient(String),
}

/// Async handler function type.
///
/// Takes extracted [`HandlerContext`] and the event payload as a JSON value.
pub type HandlerFn = Arc<
    dyn Fn(
            HandlerContext,
            serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<(), HandlerError>> + Send>>
        + Send
        + Sync,
>;

/// Result of looking up a handler in the registry.
pub enum LookupResult {
    /// Handler found for the given (event_type, schema_version).
    Found(HandlerFn),
    /// Event type not registered at all — not for this consumer, skip.
    UnknownType,
    /// Event type known but this schema version is not registered — dead-letter.
    UnknownVersion {
        /// The schema versions that ARE registered for this event type.
        known_versions: Vec<String>,
    },
}

/// Immutable handler registry mapping (event_type, schema_version) to handler functions.
///
/// Built via [`RegistryBuilder`]. Once constructed, the registry is read-only.
pub struct HandlerRegistry {
    handlers: HashMap<String, HashMap<String, HandlerFn>>,
}

impl HandlerRegistry {
    /// Look up a handler by event type and schema version.
    pub fn lookup(&self, event_type: &str, schema_version: &str) -> LookupResult {
        match self.handlers.get(event_type) {
            None => LookupResult::UnknownType,
            Some(versions) => match versions.get(schema_version) {
                Some(handler) => LookupResult::Found(Arc::clone(handler)),
                None => LookupResult::UnknownVersion {
                    known_versions: versions.keys().cloned().collect(),
                },
            },
        }
    }

    /// Returns the number of registered (event_type, schema_version) pairs.
    pub fn len(&self) -> usize {
        self.handlers.values().map(|v| v.len()).sum()
    }

    /// Returns true if no handlers are registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

/// Builder for constructing an immutable [`HandlerRegistry`].
///
/// # Example
///
/// ```rust,no_run
/// use event_consumer::{RegistryBuilder, HandlerContext, HandlerError};
///
/// let registry = RegistryBuilder::new()
///     .register("inventory.item_issued", "1.0.0", |ctx: HandlerContext, payload| async move {
///         // process the event
///         Ok(())
///     })
///     .build();
/// ```
pub struct RegistryBuilder {
    handlers: HashMap<String, HashMap<String, HandlerFn>>,
}

impl RegistryBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for the given (event_type, schema_version) pair.
    ///
    /// # Panics
    ///
    /// Panics if a handler is already registered for the same pair.
    /// This catches wiring bugs at startup rather than at runtime.
    pub fn register<F, Fut>(
        mut self,
        event_type: impl Into<String>,
        schema_version: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(HandlerContext, serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), HandlerError>> + Send + 'static,
    {
        let event_type = event_type.into();
        let schema_version = schema_version.into();

        let wrapped: HandlerFn = Arc::new(move |ctx, val| Box::pin(handler(ctx, val)));

        let versions = self.handlers.entry(event_type.clone()).or_default();
        if versions.contains_key(&schema_version) {
            panic!(
                "Duplicate handler registration: event_type={}, schema_version={}",
                event_type, schema_version
            );
        }
        versions.insert(schema_version, wrapped);

        self
    }

    /// Build the immutable registry.
    pub fn build(self) -> HandlerRegistry {
        HandlerRegistry {
            handlers: self.handlers,
        }
    }
}

impl Default for RegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> HandlerContext {
        HandlerContext {
            event_id: uuid::Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            source_module: "test".to_string(),
            correlation_id: None,
            causation_id: None,
            actor_id: None,
            schema_version: "1.0.0".to_string(),
            received_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn empty_registry() {
        let registry = RegistryBuilder::new().build();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn lookup_found() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        assert_eq!(registry.len(), 1);
        match registry.lookup("test.event", "1.0.0") {
            LookupResult::Found(handler) => {
                let result = handler(make_ctx(), serde_json::json!({})).await;
                assert!(result.is_ok());
            }
            other => panic!("Expected Found, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn lookup_unknown_type() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        assert!(matches!(
            registry.lookup("unknown.event", "1.0.0"),
            LookupResult::UnknownType
        ));
    }

    #[test]
    fn lookup_unknown_version() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        match registry.lookup("test.event", "2.0.0") {
            LookupResult::UnknownVersion { known_versions } => {
                assert_eq!(known_versions, vec!["1.0.0"]);
            }
            other => panic!(
                "Expected UnknownVersion, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn multiple_versions() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .register("test.event", "2.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        assert_eq!(registry.len(), 2);
        assert!(matches!(
            registry.lookup("test.event", "1.0.0"),
            LookupResult::Found(_)
        ));
        assert!(matches!(
            registry.lookup("test.event", "2.0.0"),
            LookupResult::Found(_)
        ));
    }

    #[test]
    fn multiple_event_types() {
        let registry = RegistryBuilder::new()
            .register("event.a", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .register("event.b", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();

        assert_eq!(registry.len(), 2);
        assert!(matches!(
            registry.lookup("event.a", "1.0.0"),
            LookupResult::Found(_)
        ));
        assert!(matches!(
            registry.lookup("event.b", "1.0.0"),
            LookupResult::Found(_)
        ));
    }

    #[test]
    #[should_panic(expected = "Duplicate handler registration")]
    fn duplicate_registration_panics() {
        let _registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .register("test.event", "1.0.0", |_ctx, _payload| async { Ok(()) })
            .build();
    }

    #[tokio::test]
    async fn handler_error_surfaces() {
        let registry = RegistryBuilder::new()
            .register("test.event", "1.0.0", |_ctx, _payload| async {
                Err(HandlerError::Permanent("bad data".to_string()))
            })
            .build();

        match registry.lookup("test.event", "1.0.0") {
            LookupResult::Found(handler) => {
                let result = handler(make_ctx(), serde_json::json!({})).await;
                assert!(result.is_err());
                let err = result.unwrap_err();
                assert!(matches!(err, HandlerError::Permanent(_)));
            }
            _ => panic!("Expected Found"),
        }
    }
}

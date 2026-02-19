//! Tracing context for propagating trace_id, correlation_id, and actor across
//! module boundaries (HTTP → service → envelope → NATS → consumer → envelope).
//!
//! # Usage
//!
//! ## At HTTP boundary (extract from headers):
//! ```rust,no_run
//! use event_bus::TracingContext;
//!
//! // From incoming HTTP request headers
//! let ctx = TracingContext::new()
//!     .with_trace_id("trace-abc".to_string())
//!     .with_correlation_id("corr-123".to_string())
//!     .with_actor(uuid::Uuid::new_v4(), "User".to_string());
//! ```
//!
//! ## At NATS consumer boundary (extract from incoming envelope):
//! ```rust,no_run
//! use event_bus::{EventEnvelope, TracingContext};
//!
//! fn consume(incoming: &EventEnvelope<serde_json::Value>) {
//!     let ctx = TracingContext::from_envelope(incoming);
//!     // ctx now carries trace_id, correlation_id, actor from the incoming event
//! }
//! ```
//!
//! ## Apply to outgoing envelope:
//! ```rust,no_run
//! use event_bus::{EventEnvelope, TracingContext};
//!
//! let ctx = TracingContext::new().with_trace_id("trace-abc".to_string());
//! let envelope = EventEnvelope::new(
//!     "tenant-1".to_string(),
//!     "my-module".to_string(),
//!     "my.event".to_string(),
//!     serde_json::json!({}),
//! )
//! .with_tracing_context(&ctx);
//! ```

use uuid::Uuid;

/// Bundles distributed tracing fields for propagation across module boundaries.
///
/// This struct is the single carrier for trace propagation context, used in:
/// - HTTP request → envelope (via header extraction)
/// - Incoming NATS envelope → outgoing envelope (via `from_envelope`)
/// - Service layer threading (pass by reference)
#[derive(Debug, Clone, Default)]
pub struct TracingContext {
    /// Distributed tracing identifier — stable across the entire request chain
    pub trace_id: Option<String>,
    /// Business transaction correlation — links related events
    pub correlation_id: Option<String>,
    /// Causation link — the event/command that triggered this one
    pub causation_id: Option<String>,
    /// Actor identifier (user, service, or system that initiated the action)
    pub actor_id: Option<Uuid>,
    /// Actor type: "User", "Service", or "System"
    pub actor_type: Option<String>,
}

impl TracingContext {
    /// Create an empty tracing context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the distributed trace ID.
    pub fn with_trace_id(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the correlation ID.
    pub fn with_correlation_id(mut self, correlation_id: String) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Set the causation ID.
    pub fn with_causation_id(mut self, causation_id: String) -> Self {
        self.causation_id = Some(causation_id);
        self
    }

    /// Set the actor identity.
    pub fn with_actor(mut self, actor_id: Uuid, actor_type: String) -> Self {
        self.actor_id = Some(actor_id);
        self.actor_type = Some(actor_type);
        self
    }

    /// Extract tracing context from an incoming EventEnvelope.
    ///
    /// This is the key propagation primitive for NATS consumers:
    /// when Module A emits an event and Module B consumes it, B calls
    /// `TracingContext::from_envelope(&incoming)` to carry forward
    /// the trace_id and actor identity.
    ///
    /// The incoming event's `event_id` becomes the `causation_id` of
    /// the downstream event (standard distributed tracing convention).
    pub fn from_envelope<T>(envelope: &super::EventEnvelope<T>) -> Self {
        Self {
            trace_id: envelope.trace_id.clone(),
            correlation_id: envelope.correlation_id.clone(),
            causation_id: Some(envelope.event_id.to_string()),
            actor_id: envelope.actor_id,
            actor_type: envelope.actor_type.clone(),
        }
    }

    /// Extract tracing context from a raw JSON envelope value.
    ///
    /// Useful in consumers that deserialize to `serde_json::Value` first.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let trace_id = value
            .get("trace_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let correlation_id = value
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let causation_id = value
            .get("event_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let actor_id = value
            .get("actor_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());

        let actor_type = value
            .get("actor_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Self {
            trace_id,
            correlation_id,
            causation_id,
            actor_id,
            actor_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventEnvelope;

    #[test]
    fn test_tracing_context_builder() {
        let actor_id = Uuid::new_v4();
        let ctx = TracingContext::new()
            .with_trace_id("trace-1".to_string())
            .with_correlation_id("corr-1".to_string())
            .with_causation_id("cause-1".to_string())
            .with_actor(actor_id, "User".to_string());

        assert_eq!(ctx.trace_id.as_deref(), Some("trace-1"));
        assert_eq!(ctx.correlation_id.as_deref(), Some("corr-1"));
        assert_eq!(ctx.causation_id.as_deref(), Some("cause-1"));
        assert_eq!(ctx.actor_id, Some(actor_id));
        assert_eq!(ctx.actor_type.as_deref(), Some("User"));
    }

    #[test]
    fn test_tracing_context_from_envelope() {
        let actor_id = Uuid::new_v4();
        let envelope = EventEnvelope::new(
            "tenant-1".to_string(),
            "source-module".to_string(),
            "test.event".to_string(),
            serde_json::json!({"data": 1}),
        )
        .with_trace_id(Some("trace-abc".to_string()))
        .with_correlation_id(Some("corr-def".to_string()))
        .with_actor(actor_id, "User".to_string());

        let ctx = TracingContext::from_envelope(&envelope);

        assert_eq!(ctx.trace_id.as_deref(), Some("trace-abc"));
        assert_eq!(ctx.correlation_id.as_deref(), Some("corr-def"));
        // causation_id should be the incoming event's event_id
        assert_eq!(ctx.causation_id, Some(envelope.event_id.to_string()));
        assert_eq!(ctx.actor_id, Some(actor_id));
        assert_eq!(ctx.actor_type.as_deref(), Some("User"));
    }

    #[test]
    fn test_tracing_context_from_envelope_sparse() {
        let envelope = EventEnvelope::new(
            "tenant-1".to_string(),
            "source-module".to_string(),
            "test.event".to_string(),
            serde_json::json!({}),
        );
        // No trace_id, no actor set

        let ctx = TracingContext::from_envelope(&envelope);

        assert!(ctx.trace_id.is_none());
        assert!(ctx.correlation_id.is_none());
        assert_eq!(ctx.causation_id, Some(envelope.event_id.to_string()));
        assert!(ctx.actor_id.is_none());
        assert!(ctx.actor_type.is_none());
    }

    #[test]
    fn test_tracing_context_from_json() {
        let actor_id = Uuid::new_v4();
        let json = serde_json::json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "trace_id": "trace-xyz",
            "correlation_id": "corr-xyz",
            "actor_id": actor_id.to_string(),
            "actor_type": "Service"
        });

        let ctx = TracingContext::from_json(&json);

        assert_eq!(ctx.trace_id.as_deref(), Some("trace-xyz"));
        assert_eq!(ctx.correlation_id.as_deref(), Some("corr-xyz"));
        assert_eq!(
            ctx.causation_id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(ctx.actor_id, Some(actor_id));
        assert_eq!(ctx.actor_type.as_deref(), Some("Service"));
    }

    #[test]
    fn test_tracing_context_default_is_empty() {
        let ctx = TracingContext::default();
        assert!(ctx.trace_id.is_none());
        assert!(ctx.correlation_id.is_none());
        assert!(ctx.causation_id.is_none());
        assert!(ctx.actor_id.is_none());
        assert!(ctx.actor_type.is_none());
    }

    #[test]
    fn test_tracing_context_roundtrip_envelope() {
        // Simulate: Module A creates envelope → Module B consumes → applies to new envelope
        let actor_id = Uuid::new_v4();
        let original = EventEnvelope::new(
            "tenant-1".to_string(),
            "module-a".to_string(),
            "a.event".to_string(),
            serde_json::json!({"data": 1}),
        )
        .with_trace_id(Some("trace-roundtrip".to_string()))
        .with_correlation_id(Some("corr-roundtrip".to_string()))
        .with_actor(actor_id, "User".to_string())
        .with_mutation_class(Some("DATA_MUTATION".to_string()));

        // Module B extracts context
        let ctx = TracingContext::from_envelope(&original);

        // Module B creates downstream envelope with propagated context
        let downstream = EventEnvelope::new(
            "tenant-1".to_string(),
            "module-b".to_string(),
            "b.downstream".to_string(),
            serde_json::json!({"derived": true}),
        )
        .with_tracing_context(&ctx)
        .with_mutation_class(Some("DATA_MUTATION".to_string()));

        // Verify propagation
        assert_eq!(downstream.trace_id, original.trace_id);
        assert_eq!(downstream.correlation_id, original.correlation_id);
        assert_eq!(
            downstream.causation_id,
            Some(original.event_id.to_string())
        );
        assert_eq!(downstream.actor_id, original.actor_id);
        assert_eq!(downstream.actor_type, original.actor_type);
        // Module B's own fields are preserved
        assert_eq!(downstream.source_module, "module-b");
        assert_eq!(downstream.event_type, "b.downstream");
    }
}

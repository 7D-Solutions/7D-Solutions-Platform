//! Builder pattern methods for EventEnvelope.
//!
//! Provides fluent API for setting optional envelope fields after construction.

use super::EventEnvelope;
use uuid::Uuid;

impl<T> EventEnvelope<T> {
    /// Set the source version
    pub fn with_source_version(mut self, version: String) -> Self {
        self.source_version = version;
        self
    }

    /// Set the schema version
    pub fn with_schema_version(mut self, version: String) -> Self {
        self.schema_version = version;
        self
    }

    /// Set the trace ID
    pub fn with_trace_id(mut self, trace_id: Option<String>) -> Self {
        self.trace_id = trace_id;
        self
    }

    /// Set the correlation ID
    pub fn with_correlation_id(mut self, correlation_id: Option<String>) -> Self {
        self.correlation_id = correlation_id;
        self
    }

    /// Set the causation ID
    pub fn with_causation_id(mut self, causation_id: Option<String>) -> Self {
        self.causation_id = causation_id;
        self
    }

    /// Set the reverses event ID (for compensating transactions)
    pub fn with_reverses_event_id(mut self, reverses_event_id: Option<Uuid>) -> Self {
        self.reverses_event_id = reverses_event_id;
        self
    }

    /// Set the supersedes event ID (for corrections)
    pub fn with_supersedes_event_id(mut self, supersedes_event_id: Option<Uuid>) -> Self {
        self.supersedes_event_id = supersedes_event_id;
        self
    }

    /// Set the side effect ID
    pub fn with_side_effect_id(mut self, side_effect_id: Option<String>) -> Self {
        self.side_effect_id = side_effect_id;
        self
    }

    /// Set the replay safe flag
    pub fn with_replay_safe(mut self, replay_safe: bool) -> Self {
        self.replay_safe = replay_safe;
        self
    }

    /// Set the mutation class
    pub fn with_mutation_class(mut self, mutation_class: Option<String>) -> Self {
        self.mutation_class = mutation_class;
        self
    }

    /// Set the actor identity
    pub fn with_actor(mut self, actor_id: Uuid, actor_type: String) -> Self {
        self.actor_id = Some(actor_id);
        self.actor_type = Some(actor_type);
        self
    }

    /// Set the actor identity from audit::Actor
    pub fn with_actor_from(mut self, actor_id: Option<Uuid>, actor_type: Option<String>) -> Self {
        self.actor_id = actor_id;
        self.actor_type = actor_type;
        self
    }
}

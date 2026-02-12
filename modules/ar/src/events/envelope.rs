use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// EventEnvelope wraps AR domain events with metadata for event-driven integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    /// Unique event identifier (idempotency key)
    pub event_id: Uuid,
    /// Event type in format: domain.entity.action (e.g., ar.invoice.created)
    pub event_type: String,
    /// Schema version for this event type
    pub schema_version: String,
    /// When the event occurred (ISO 8601)
    pub occurred_at: DateTime<Utc>,
    /// Module that produced this event
    pub producer: String,
    /// Tenant identifier for multi-tenant isolation
    pub tenant_id: String,
    /// Aggregate type (e.g., "invoice", "payment")
    pub aggregate_type: String,
    /// Aggregate instance ID
    pub aggregate_id: String,
    /// Trace ID for distributed tracing
    pub trace_id: String,
    /// Event that caused this event (for causality tracking)
    pub causation_id: Option<Uuid>,
    /// The actual event data
    pub data: T,
}

impl<T> EventEnvelope<T> {
    /// Create a new event envelope
    pub fn new(
        event_type: String,
        schema_version: String,
        tenant_id: String,
        aggregate_type: String,
        aggregate_id: String,
        trace_id: String,
        data: T,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            event_type,
            schema_version,
            occurred_at: Utc::now(),
            producer: "ar-rs".to_string(),
            tenant_id,
            aggregate_type,
            aggregate_id,
            trace_id,
            causation_id: None,
            data,
        }
    }

    /// Set causation_id to track event causality
    pub fn with_causation(mut self, causation_id: Uuid) -> Self {
        self.causation_id = Some(causation_id);
        self
    }
}

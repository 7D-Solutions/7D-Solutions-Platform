use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Standard event envelope following platform event contract
/// See: contracts/events/README.md
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    /// Unique event identifier (idempotency key)
    pub event_id: Uuid,

    /// ISO 8601 timestamp when event was generated
    pub occurred_at: DateTime<Utc>,

    /// Tenant identifier for multi-tenant isolation
    pub tenant_id: String,

    /// Module that generated the event (always "payments" for this module)
    pub source_module: String,

    /// Semantic version of the source module
    pub source_version: String,

    /// Links related events in a business transaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,

    /// Links this event to the command/event that caused it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,

    /// Event-specific payload
    pub payload: T,
}

impl<T> EventEnvelope<T> {
    /// Create a new event envelope
    pub fn new(
        event_id: Uuid,
        tenant_id: String,
        correlation_id: Option<String>,
        causation_id: Option<String>,
        payload: T,
    ) -> Self {
        Self {
            event_id,
            occurred_at: Utc::now(),
            tenant_id,
            source_module: "payments".to_string(),
            source_version: env!("CARGO_PKG_VERSION").to_string(),
            correlation_id,
            causation_id,
            payload,
        }
    }
}

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Metadata extracted from an [`event_bus::EventEnvelope`] for handler consumption.
///
/// The router builds this from the incoming envelope before dispatching to
/// the handler, so handlers never need to parse raw envelope fields.
#[derive(Debug, Clone)]
pub struct HandlerContext {
    /// Unique event identifier (idempotency key).
    pub event_id: Uuid,
    /// Tenant that owns this event.
    pub tenant_id: String,
    /// Module that produced the event (e.g. "inventory", "production").
    pub source_module: String,
    /// Business-transaction correlation identifier.
    pub correlation_id: Option<String>,
    /// Identifier of the command/event that caused this one.
    pub causation_id: Option<String>,
    /// Actor who triggered the event.
    pub actor_id: Option<Uuid>,
    /// Schema version of the payload.
    pub schema_version: String,
    /// Timestamp when the consumer received the message.
    pub received_at: DateTime<Utc>,
}

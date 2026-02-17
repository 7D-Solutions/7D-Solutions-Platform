//! # Event Envelope
//!
//! Platform-wide event envelope specification for all inter-module communication.
//!
//! ## Design Principles
//!
//! 1. **Single Source of Truth**: One envelope struct for the entire platform
//! 2. **Validation**: Centralized envelope validation logic
//! 3. **Versioning**: Supports module versioning for safe evolution
//! 4. **Tracing**: Built-in support for distributed tracing and causality
//!
//! ## Envelope Fields
//!
//! - `event_id`: Unique identifier for idempotency
//! - `event_type`: Type/name of the event (e.g., "payment.succeeded")
//! - `occurred_at`: ISO 8601 timestamp when event was generated
//! - `tenant_id`: Multi-tenant isolation
//! - `source_module`: Module that produced the event
//! - `source_version`: Semantic version of the source module
//! - `schema_version`: Version of the payload schema
//! - `trace_id`: Distributed tracing identifier
//! - `correlation_id`: Links related events in a business transaction
//! - `causation_id`: Links this event to the command/event that caused it
//! - `reverses_event_id`: Points to the event being reversed (for compensating transactions)
//! - `supersedes_event_id`: Points to the event being superseded (for corrections)
//! - `side_effect_id`: Tracks side effects for idempotency
//! - `replay_safe`: Indicates if the event can be safely replayed
//! - `mutation_class`: Classification of the mutation (e.g., "financial", "user-data")
//! - `actor_id`: Identifier of the actor who caused this event
//! - `actor_type`: Type of actor (User, Service, System)
//! - `payload`: Event-specific data (generic type parameter)

mod builder;
mod validation;

#[cfg(test)]
mod tests;

pub use validation::validate_envelope_fields;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Standard event envelope following platform event contract
///
/// This envelope wraps all events published across module boundaries.
/// It provides metadata for idempotency, tracing, and multi-tenancy.
///
/// # Type Parameter
///
/// * `T` - The event-specific payload type
///
/// # Examples
///
/// ```rust
/// use event_bus::EventEnvelope;
/// use serde::{Deserialize, Serialize};
/// use uuid::Uuid;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct PaymentSucceeded {
///     payment_id: String,
///     amount: i64,
///     currency: String,
/// }
///
/// let envelope = EventEnvelope::new(
///     "tenant-123".to_string(),
///     "payments".to_string(),
///     "payment.succeeded".to_string(),
///     PaymentSucceeded {
///         payment_id: "pay_123".to_string(),
///         amount: 1000,
///         currency: "usd".to_string(),
///     },
/// )
/// .with_correlation_id(Some("correlation-456".to_string()))
/// .with_mutation_class(Some("financial".to_string()));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    /// Unique event identifier (idempotency key)
    pub event_id: Uuid,

    /// Type/name of the event (e.g., "payment.succeeded", "invoice.created")
    pub event_type: String,

    /// ISO 8601 timestamp when event was generated
    pub occurred_at: DateTime<Utc>,

    /// Tenant identifier for multi-tenant isolation
    pub tenant_id: String,

    /// Module that generated the event (e.g., "ar", "payments", "subscriptions")
    pub source_module: String,

    /// Semantic version of the source module
    pub source_version: String,

    /// Version of the payload schema
    pub schema_version: String,

    /// Distributed tracing identifier for end-to-end request tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,

    /// Links related events in a business transaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,

    /// Links this event to the command/event that caused it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,

    /// Points to the event being reversed (for compensating transactions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverses_event_id: Option<Uuid>,

    /// Points to the event being superseded (for corrections)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_event_id: Option<Uuid>,

    /// Tracks side effects for idempotency
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side_effect_id: Option<String>,

    /// Indicates if the event can be safely replayed
    pub replay_safe: bool,

    /// Classification of the mutation (e.g., "financial", "user-data")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_class: Option<String>,

    /// Identifier of the actor who caused this event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<Uuid>,

    /// Type of actor (User, Service, System)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_type: Option<String>,

    /// Event-specific payload
    pub payload: T,
}

impl<T> EventEnvelope<T> {
    /// Create a new event envelope
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `source_module` - Module producing the event
    /// * `event_type` - Type/name of the event
    /// * `payload` - Event-specific data
    ///
    /// # Returns
    ///
    /// A new envelope with auto-generated event_id and occurred_at
    ///
    /// # Note
    ///
    /// The source_version and schema_version default to "1.0.0".
    /// The replay_safe defaults to true.
    /// Callers should pass the module name (e.g., "ar", "payments").
    pub fn new(tenant_id: String, source_module: String, event_type: String, payload: T) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            event_type,
            occurred_at: Utc::now(),
            tenant_id,
            source_module,
            source_version: "1.0.0".to_string(), // Default, should be overridden by caller
            schema_version: "1.0.0".to_string(), // Default, should be overridden by caller
            trace_id: None,
            correlation_id: None,
            causation_id: None,
            reverses_event_id: None,
            supersedes_event_id: None,
            side_effect_id: None,
            replay_safe: true, // Safe default
            mutation_class: None,
            actor_id: None,
            actor_type: None,
            payload,
        }
    }

    /// Create an envelope with explicit event_id (useful for testing)
    pub fn with_event_id(
        event_id: Uuid,
        tenant_id: String,
        source_module: String,
        event_type: String,
        payload: T,
    ) -> Self {
        Self {
            event_id,
            event_type,
            occurred_at: Utc::now(),
            tenant_id,
            source_module,
            source_version: "1.0.0".to_string(),
            schema_version: "1.0.0".to_string(),
            trace_id: None,
            correlation_id: None,
            causation_id: None,
            reverses_event_id: None,
            supersedes_event_id: None,
            side_effect_id: None,
            replay_safe: true,
            mutation_class: None,
            actor_id: None,
            actor_type: None,
            payload,
        }
    }
}

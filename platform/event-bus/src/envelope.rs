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
//! - `occurred_at`: ISO 8601 timestamp when event was generated
//! - `tenant_id`: Multi-tenant isolation
//! - `source_module`: Module that produced the event
//! - `source_version`: Semantic version of the source module
//! - `correlation_id`: Links related events in a business transaction
//! - `causation_id`: Links this event to the command/event that caused it
//! - `payload`: Event-specific data (generic type parameter)

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
///     PaymentSucceeded {
///         payment_id: "pay_123".to_string(),
///         amount: 1000,
///         currency: "usd".to_string(),
///     },
/// )
/// .with_correlation_id(Some("correlation-456".to_string()));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    /// Unique event identifier (idempotency key)
    pub event_id: Uuid,

    /// ISO 8601 timestamp when event was generated
    pub occurred_at: DateTime<Utc>,

    /// Tenant identifier for multi-tenant isolation
    pub tenant_id: String,

    /// Module that generated the event (e.g., "ar", "payments", "subscriptions")
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
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `source_module` - Module producing the event
    /// * `payload` - Event-specific data
    ///
    /// # Returns
    ///
    /// A new envelope with auto-generated event_id and occurred_at
    ///
    /// # Note
    ///
    /// The source_version is automatically set to the module's CARGO_PKG_VERSION.
    /// Callers should pass the module name (e.g., "ar", "payments").
    pub fn new(tenant_id: String, source_module: String, payload: T) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            tenant_id,
            source_module,
            source_version: "1.0.0".to_string(), // Default, should be overridden by caller
            correlation_id: None,
            causation_id: None,
            payload,
        }
    }

    /// Create an envelope with explicit event_id (useful for testing)
    pub fn with_event_id(
        event_id: Uuid,
        tenant_id: String,
        source_module: String,
        payload: T,
    ) -> Self {
        Self {
            event_id,
            occurred_at: Utc::now(),
            tenant_id,
            source_module,
            source_version: "1.0.0".to_string(),
            correlation_id: None,
            causation_id: None,
            payload,
        }
    }

    /// Set the source version
    pub fn with_source_version(mut self, version: String) -> Self {
        self.source_version = version;
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
}

/// Validate an event envelope (generic payload)
///
/// # Validation Rules
///
/// - `event_id`: Must be a valid UUID
/// - `occurred_at`: Must be present
/// - `tenant_id`: Must be non-empty
/// - `source_module`: Must be non-empty
/// - `source_version`: Must be non-empty
///
/// # Errors
///
/// Returns a descriptive error string if validation fails
pub fn validate_envelope_fields(envelope: &serde_json::Value) -> Result<(), String> {
    // Validate event_id
    envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid event_id")?;

    // Validate occurred_at
    envelope
        .get("occurred_at")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid occurred_at")?;

    // Validate tenant_id
    let tenant_id = envelope
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid tenant_id")?;

    if tenant_id.is_empty() {
        return Err("tenant_id cannot be empty".to_string());
    }

    // Validate source_module
    let source_module = envelope
        .get("source_module")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid source_module")?;

    if source_module.is_empty() {
        return Err("source_module cannot be empty".to_string());
    }

    // Validate source_version
    let source_version = envelope
        .get("source_module")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid source_version")?;

    if source_version.is_empty() {
        return Err("source_version cannot be empty".to_string());
    }

    // correlation_id and causation_id are optional
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_envelope_creation() {
        let envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            json!({"test": "data"}),
        );

        assert_eq!(envelope.tenant_id, "tenant-123");
        assert_eq!(envelope.source_module, "test-module");
        assert!(envelope.correlation_id.is_none());
        assert!(envelope.causation_id.is_none());
    }

    #[test]
    fn test_envelope_with_builder() {
        let envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            json!({"test": "data"}),
        )
        .with_source_version("1.2.3".to_string())
        .with_correlation_id(Some("corr-456".to_string()))
        .with_causation_id(Some("cause-789".to_string()));

        assert_eq!(envelope.source_version, "1.2.3");
        assert_eq!(envelope.correlation_id, Some("corr-456".to_string()));
        assert_eq!(envelope.causation_id, Some("cause-789".to_string()));
    }

    #[test]
    fn test_validate_envelope_fields_valid() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "payload": {}
        });

        assert!(validate_envelope_fields(&envelope).is_ok());
    }

    #[test]
    fn test_validate_envelope_fields_missing_tenant_id() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2024-01-01T00:00:00Z",
            "source_module": "payments",
            "source_version": "1.0.0"
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_empty_tenant_id() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "",
            "source_module": "payments",
            "source_version": "1.0.0"
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }
}

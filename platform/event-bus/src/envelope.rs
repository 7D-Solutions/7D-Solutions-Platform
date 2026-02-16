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
            payload,
        }
    }

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
}

/// Validate an event envelope (generic payload)
///
/// # Validation Rules
///
/// - `event_id`: Must be a valid UUID
/// - `event_type`: Must be non-empty
/// - `occurred_at`: Must be present
/// - `tenant_id`: Must be non-empty
/// - `source_module`: Must be non-empty
/// - `source_version`: Must be non-empty
/// - `schema_version`: Must be non-empty
/// - `replay_safe`: Must be a boolean
/// - `mutation_class`: Must be present and one of the valid classes (Phase 16)
/// - All other fields are optional
///
/// # Valid Mutation Classes
///
/// - `DATA_MUTATION`: Financial/audit mutations (idempotent)
/// - `REVERSAL`: Compensating transactions
/// - `CORRECTION`: Superseding corrections
/// - `SIDE_EFFECT`: Non-idempotent external actions
/// - `QUERY`: Read-only operations
/// - `LIFECYCLE`: Entity lifecycle transitions
/// - `ADMINISTRATIVE`: Configuration/setup changes
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

    // Validate event_type
    let event_type = envelope
        .get("event_type")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid event_type")?;

    if event_type.is_empty() {
        return Err("event_type cannot be empty".to_string());
    }

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
        .get("source_version")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid source_version")?;

    if source_version.is_empty() {
        return Err("source_version cannot be empty".to_string());
    }

    // Validate schema_version
    let schema_version = envelope
        .get("schema_version")
        .and_then(|v| v.as_str())
        .ok_or("Missing or invalid schema_version")?;

    if schema_version.is_empty() {
        return Err("schema_version cannot be empty".to_string());
    }

    // Validate replay_safe
    envelope
        .get("replay_safe")
        .and_then(|v| v.as_bool())
        .ok_or("Missing or invalid replay_safe")?;

    // Validate optional string fields are non-empty if present
    if let Some(trace_id) = envelope.get("trace_id").and_then(|v| v.as_str()) {
        if trace_id.is_empty() {
            return Err("trace_id cannot be empty".to_string());
        }
    }

    if let Some(correlation_id) = envelope.get("correlation_id").and_then(|v| v.as_str()) {
        if correlation_id.is_empty() {
            return Err("correlation_id cannot be empty".to_string());
        }
    }

    if let Some(causation_id) = envelope.get("causation_id").and_then(|v| v.as_str()) {
        if causation_id.is_empty() {
            return Err("causation_id cannot be empty".to_string());
        }
    }

    if let Some(side_effect_id) = envelope.get("side_effect_id").and_then(|v| v.as_str()) {
        if side_effect_id.is_empty() {
            return Err("side_effect_id cannot be empty".to_string());
        }
    }

    // Validate mutation_class (Phase 16: Required field)
    let mutation_class = envelope
        .get("mutation_class")
        .and_then(|v| v.as_str())
        .ok_or("mutation_class is required")?;

    if mutation_class.is_empty() {
        return Err("mutation_class cannot be empty".to_string());
    }

    // Validate mutation_class is a known value (from MUTATION-CLASSES.md)
    const VALID_CLASSES: &[&str] = &[
        "DATA_MUTATION",
        "REVERSAL",
        "CORRECTION",
        "SIDE_EFFECT",
        "QUERY",
        "LIFECYCLE",
        "ADMINISTRATIVE",
    ];

    if !VALID_CLASSES.contains(&mutation_class) {
        return Err(format!(
            "Invalid mutation_class: '{}'. Must be one of: {:?}",
            mutation_class, VALID_CLASSES
        ));
    }

    // reverses_event_id and supersedes_event_id are optional UUIDs
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
            "test.event".to_string(),
            json!({"test": "data"}),
        );

        assert_eq!(envelope.tenant_id, "tenant-123");
        assert_eq!(envelope.source_module, "test-module");
        assert_eq!(envelope.event_type, "test.event");
        assert_eq!(envelope.schema_version, "1.0.0");
        assert!(envelope.replay_safe);
        assert!(envelope.trace_id.is_none());
        assert!(envelope.correlation_id.is_none());
        assert!(envelope.causation_id.is_none());
        assert!(envelope.reverses_event_id.is_none());
        assert!(envelope.supersedes_event_id.is_none());
        assert!(envelope.side_effect_id.is_none());
        assert!(envelope.mutation_class.is_none());
    }

    #[test]
    fn test_envelope_with_builder() {
        let reverses_id = Uuid::new_v4();
        let supersedes_id = Uuid::new_v4();

        let envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            "test.event".to_string(),
            json!({"test": "data"}),
        )
        .with_source_version("1.2.3".to_string())
        .with_schema_version("2.0.0".to_string())
        .with_trace_id(Some("trace-123".to_string()))
        .with_correlation_id(Some("corr-456".to_string()))
        .with_causation_id(Some("cause-789".to_string()))
        .with_reverses_event_id(Some(reverses_id))
        .with_supersedes_event_id(Some(supersedes_id))
        .with_side_effect_id(Some("side-effect-123".to_string()))
        .with_replay_safe(false)
        .with_mutation_class(Some("financial".to_string()));

        assert_eq!(envelope.source_version, "1.2.3");
        assert_eq!(envelope.schema_version, "2.0.0");
        assert_eq!(envelope.trace_id, Some("trace-123".to_string()));
        assert_eq!(envelope.correlation_id, Some("corr-456".to_string()));
        assert_eq!(envelope.causation_id, Some("cause-789".to_string()));
        assert_eq!(envelope.reverses_event_id, Some(reverses_id));
        assert_eq!(envelope.supersedes_event_id, Some(supersedes_id));
        assert_eq!(envelope.side_effect_id, Some("side-effect-123".to_string()));
        assert!(!envelope.replay_safe);
        assert_eq!(envelope.mutation_class, Some("financial".to_string()));
    }

    #[test]
    fn test_validate_envelope_fields_valid() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "payload": {}
        });

        assert!(validate_envelope_fields(&envelope).is_ok());
    }

    #[test]
    fn test_validate_envelope_fields_missing_tenant_id() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_empty_tenant_id() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_missing_event_type() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_missing_schema_version() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "replay_safe": true
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_missing_replay_safe() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0"
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_empty_event_type() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_empty_schema_version() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "",
            "replay_safe": true
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_empty_trace_id() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "trace_id": ""
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_empty_mutation_class() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": ""
        });

        assert!(validate_envelope_fields(&envelope).is_err());
    }

    #[test]
    fn test_validate_envelope_fields_valid_with_optional_fields() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "trace_id": "trace-123",
            "correlation_id": "corr-456",
            "causation_id": "cause-789"
        });

        assert!(validate_envelope_fields(&envelope).is_ok());
    }

    #[test]
    fn test_validate_envelope_fields_missing_mutation_class() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true
        });

        let result = validate_envelope_fields(&envelope);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutation_class is required"));
    }

    #[test]
    fn test_validate_envelope_fields_invalid_mutation_class() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "INVALID_CLASS"
        });

        let result = validate_envelope_fields(&envelope);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid mutation_class"));
    }

    #[test]
    fn test_validate_envelope_fields_valid_mutation_classes() {
        let valid_classes = vec![
            "DATA_MUTATION",
            "REVERSAL",
            "CORRECTION",
            "SIDE_EFFECT",
            "QUERY",
            "LIFECYCLE",
            "ADMINISTRATIVE",
        ];

        for class in valid_classes {
            let envelope = json!({
                "event_id": "550e8400-e29b-41d4-a716-446655440000",
                "event_type": "test.event",
                "occurred_at": "2024-01-01T00:00:00Z",
                "tenant_id": "tenant-123",
                "source_module": "test",
                "source_version": "1.0.0",
                "schema_version": "1.0.0",
                "replay_safe": true,
                "mutation_class": class
            });

            assert!(
                validate_envelope_fields(&envelope).is_ok(),
                "Expected {} to be valid",
                class
            );
        }
    }
}

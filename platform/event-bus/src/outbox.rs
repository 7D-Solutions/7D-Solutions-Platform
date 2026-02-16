//! # Outbox Validation
//!
//! Platform-level outbox validation to ensure all events meet constitutional requirements
//! before being enqueued for publishing.
//!
//! ## Design Principle
//!
//! **Invariant**: No outbox insert occurs without passing envelope validation.
//!
//! **Failure Mode to Avoid**: Bypass paths emitting non-compliant events.
//!
//! ## Usage
//!
//! All module outbox enqueue functions MUST route through `validate_and_serialize_envelope`
//! before inserting into their outbox tables. This ensures:
//!
//! 1. Constitutional envelope metadata is present and non-empty
//! 2. Required fields are validated before persistence
//! 3. Invalid envelopes are rejected at the boundary, not at publish time
//!
//! # Examples
//!
//! ```rust,no_run
//! use event_bus::{EventEnvelope, outbox::validate_and_serialize_envelope};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct MyEvent {
//!     field: String,
//! }
//!
//! async fn enqueue_event(envelope: &EventEnvelope<MyEvent>) -> Result<(), String> {
//!     // Validate and serialize before outbox insert
//!     let validated_payload = validate_and_serialize_envelope(envelope)?;
//!
//!     // Now safe to insert into outbox
//!     // sqlx::query(...).bind(validated_payload).execute(...).await?;
//!     Ok(())
//! }
//! ```

use crate::envelope::{validate_envelope_fields, EventEnvelope};
use serde::Serialize;

/// Validate an EventEnvelope and serialize it to JSON
///
/// This is the **required boundary** for all outbox enqueue operations.
/// Modules MUST call this function before inserting events into their outbox tables.
///
/// # Arguments
///
/// * `envelope` - The event envelope to validate and serialize
///
/// # Returns
///
/// * `Ok(serde_json::Value)` - Validated and serialized envelope ready for storage
/// * `Err(String)` - Validation error with descriptive message
///
/// # Errors
///
/// Returns an error if:
/// - Required fields are missing or empty
/// - Optional string fields are present but empty
/// - Serialization fails
///
/// # Example
///
/// ```rust,no_run
/// use event_bus::{EventEnvelope, outbox::validate_and_serialize_envelope};
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct PaymentEvent {
///     amount: i64,
/// }
///
/// async fn example() -> Result<(), String> {
///     let envelope = EventEnvelope::new(
///         "tenant-123".to_string(),
///         "payments".to_string(),
///         "payment.succeeded".to_string(),
///         PaymentEvent { amount: 1000 },
///     );
///
///     let validated_payload = validate_and_serialize_envelope(&envelope)?;
///     // Now insert into outbox with validated_payload
///     Ok(())
/// }
/// ```
pub fn validate_and_serialize_envelope<T: Serialize>(
    envelope: &EventEnvelope<T>,
) -> Result<serde_json::Value, String> {
    // Serialize the envelope to JSON
    let payload = serde_json::to_value(envelope)
        .map_err(|e| format!("Failed to serialize envelope: {}", e))?;

    // Validate envelope fields
    validate_envelope_fields(&payload)?;

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct TestPayload {
        test_field: String,
    }

    #[test]
    fn test_validate_and_serialize_envelope_valid() {
        let envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            "test.event".to_string(),
            TestPayload {
                test_field: "data".to_string(),
            },
        )
        .with_source_version("1.2.3".to_string())
        .with_schema_version("2.0.0".to_string());

        let result = validate_and_serialize_envelope(&envelope);
        assert!(result.is_ok());

        let payload = result.unwrap();
        assert_eq!(payload["tenant_id"], "tenant-123");
        assert_eq!(payload["source_module"], "test-module");
        assert_eq!(payload["event_type"], "test.event");
        assert_eq!(payload["source_version"], "1.2.3");
        assert_eq!(payload["schema_version"], "2.0.0");
        assert_eq!(payload["replay_safe"], true);
    }

    #[test]
    fn test_validate_and_serialize_envelope_with_optional_fields() {
        let envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            "test.event".to_string(),
            TestPayload {
                test_field: "data".to_string(),
            },
        )
        .with_trace_id(Some("trace-123".to_string()))
        .with_correlation_id(Some("corr-456".to_string()))
        .with_mutation_class(Some("financial".to_string()));

        let result = validate_and_serialize_envelope(&envelope);
        assert!(result.is_ok());

        let payload = result.unwrap();
        assert_eq!(payload["trace_id"], "trace-123");
        assert_eq!(payload["correlation_id"], "corr-456");
        assert_eq!(payload["mutation_class"], "financial");
    }

    #[test]
    fn test_validate_and_serialize_envelope_rejects_empty_trace_id() {
        // Create envelope with empty trace_id (bypassing constructor)
        let mut envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            "test.event".to_string(),
            TestPayload {
                test_field: "data".to_string(),
            },
        );
        envelope.trace_id = Some("".to_string()); // Manually set empty string

        let result = validate_and_serialize_envelope(&envelope);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("trace_id cannot be empty"));
    }

    #[test]
    fn test_validate_and_serialize_envelope_rejects_empty_mutation_class() {
        // Create envelope with empty mutation_class (bypassing constructor)
        let mut envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            "test.event".to_string(),
            TestPayload {
                test_field: "data".to_string(),
            },
        );
        envelope.mutation_class = Some("".to_string()); // Manually set empty string

        let result = validate_and_serialize_envelope(&envelope);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("mutation_class cannot be empty"));
    }

    #[test]
    fn test_validate_and_serialize_envelope_accepts_none_optional_fields() {
        let envelope = EventEnvelope::new(
            "tenant-123".to_string(),
            "test-module".to_string(),
            "test.event".to_string(),
            TestPayload {
                test_field: "data".to_string(),
            },
        );

        // All optional fields are None - should be valid
        let result = validate_and_serialize_envelope(&envelope);
        assert!(result.is_ok());
    }
}

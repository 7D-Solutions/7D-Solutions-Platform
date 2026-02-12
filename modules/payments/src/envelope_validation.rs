use chrono::{DateTime, Utc};
use serde_json::Value;
use std::env;
use uuid::Uuid;

/// Validates the envelope fields according to the platform event contract
///
/// This function checks that all required envelope fields are present and valid:
/// - event_id: valid UUID
/// - occurred_at: valid ISO 8601 timestamp
/// - tenant_id: non-empty string
/// - source_module: non-empty string
/// - source_version: non-empty string
/// - payload: object
///
/// Optional fields (correlation_id, causation_id) are validated if present.
pub fn validate_envelope(envelope: &Value) -> Result<(), String> {
    // Validate event_id (required UUID)
    let event_id = envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: event_id")?;

    Uuid::parse_str(event_id)
        .map_err(|_| format!("Invalid event_id: must be a valid UUID, got '{}'", event_id))?;

    // Validate occurred_at (required ISO 8601 timestamp)
    let occurred_at = envelope
        .get("occurred_at")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: occurred_at")?;

    DateTime::parse_from_rfc3339(occurred_at)
        .map_err(|_| format!("Invalid occurred_at: must be ISO 8601 timestamp, got '{}'", occurred_at))?;

    // Validate tenant_id (required non-empty string)
    let tenant_id = envelope
        .get("tenant_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: tenant_id")?;

    if tenant_id.trim().is_empty() {
        return Err("Invalid tenant_id: must be non-empty".to_string());
    }

    // Validate source_module or producer (required non-empty string)
    let source_module = envelope
        .get("source_module")
        .or_else(|| envelope.get("producer"))
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: source_module (or producer)")?;

    if source_module.trim().is_empty() {
        return Err("Invalid source_module/producer: must be non-empty".to_string());
    }

    // Validate source_version or schema_version (required non-empty string)
    let source_version = envelope
        .get("source_version")
        .or_else(|| envelope.get("schema_version"))
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: source_version (or schema_version)")?;

    if source_version.trim().is_empty() {
        return Err("Invalid source_version/schema_version: must be non-empty".to_string());
    }

    // Validate correlation_id or trace_id (optional string)
    let corr_or_trace = envelope.get("correlation_id").or_else(|| envelope.get("trace_id"));
    if let Some(val) = corr_or_trace {
        if !val.is_string() && !val.is_null() {
            return Err("Invalid correlation_id/trace_id: must be a string or null".to_string());
        }
    }

    // Validate causation_id (optional string)
    if let Some(causation_id) = envelope.get("causation_id") {
        if !causation_id.is_string() && !causation_id.is_null() {
            return Err("Invalid causation_id: must be a string or null".to_string());
        }
    }

    // Validate payload or data (required object)
    let payload = envelope
        .get("payload")
        .or_else(|| envelope.get("data"))
        .ok_or("Missing required field: payload (or data)")?;

    if !payload.is_object() {
        return Err("Invalid payload/data: must be an object".to_string());
    }

    // Optional schema validation if enabled
    if env::var("ENABLE_SCHEMA_VALIDATION").unwrap_or_default() == "true" {
        // Schema validation would be implemented here
        // For now, we'll just log that it's enabled
        tracing::debug!("Schema validation enabled but not yet implemented");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_envelope() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0",
            "payload": {
                "invoice_id": "inv-123"
            }
        });

        assert!(validate_envelope(&envelope).is_ok());
    }

    #[test]
    fn test_valid_envelope_with_optional_fields() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0",
            "correlation_id": "corr-123",
            "causation_id": "cause-123",
            "payload": {
                "invoice_id": "inv-123"
            }
        });

        assert!(validate_envelope(&envelope).is_ok());
    }

    #[test]
    fn test_missing_event_id() {
        let envelope = json!({
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0",
            "payload": {}
        });

        assert!(validate_envelope(&envelope).is_err());
        assert_eq!(
            validate_envelope(&envelope).unwrap_err(),
            "Missing required field: event_id"
        );
    }

    #[test]
    fn test_invalid_event_id() {
        let envelope = json!({
            "event_id": "not-a-uuid",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0",
            "payload": {}
        });

        assert!(validate_envelope(&envelope).is_err());
        assert!(validate_envelope(&envelope).unwrap_err().contains("Invalid event_id"));
    }

    #[test]
    fn test_invalid_occurred_at() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "not-a-timestamp",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0",
            "payload": {}
        });

        assert!(validate_envelope(&envelope).is_err());
        assert!(validate_envelope(&envelope).unwrap_err().contains("Invalid occurred_at"));
    }

    #[test]
    fn test_empty_tenant_id() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "   ",
            "source_module": "ar",
            "source_version": "1.0.0",
            "payload": {}
        });

        assert!(validate_envelope(&envelope).is_err());
        assert_eq!(
            validate_envelope(&envelope).unwrap_err(),
            "Invalid tenant_id: must be non-empty"
        );
    }

    #[test]
    fn test_missing_payload() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0"
        });

        assert!(validate_envelope(&envelope).is_err());
        assert_eq!(
            validate_envelope(&envelope).unwrap_err(),
            "Missing required field: payload (or data)"
        );
    }

    #[test]
    fn test_invalid_payload_type() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "ar",
            "source_version": "1.0.0",
            "payload": "not-an-object"
        });

        assert!(validate_envelope(&envelope).is_err());
        assert_eq!(
            validate_envelope(&envelope).unwrap_err(),
            "Invalid payload/data: must be an object"
        );
    }

    #[test]
    fn test_valid_ar_envelope() {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "producer": "ar-rs",
            "schema_version": "1.0.0",
            "trace_id": "trace-123",
            "data": {
                "invoice_id": "inv-123"
            }
        });

        assert!(validate_envelope(&envelope).is_ok());
    }
}

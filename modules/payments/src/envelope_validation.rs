use chrono::DateTime;
use serde_json::Value;
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

    DateTime::parse_from_rfc3339(occurred_at).map_err(|_| {
        format!(
            "Invalid occurred_at: must be ISO 8601 timestamp, got '{}'",
            occurred_at
        )
    })?;

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
    let corr_or_trace = envelope
        .get("correlation_id")
        .or_else(|| envelope.get("trace_id"));
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

    // Delegate to platform envelope contract validation (constitutional fields)
    event_bus::validate_envelope_fields(envelope)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_envelope() -> serde_json::Value {
        json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "payment.succeeded",
            "occurred_at": "2026-02-12T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "payload": {
                "payment_id": "pay-123"
            }
        })
    }

    #[test]
    fn test_valid_envelope() {
        assert!(validate_envelope(&valid_envelope()).is_ok());
    }

    #[test]
    fn test_valid_envelope_with_optional_fields() {
        let mut envelope = valid_envelope();
        envelope["correlation_id"] = json!("corr-123");
        envelope["causation_id"] = json!("cause-123");
        assert!(validate_envelope(&envelope).is_ok());
    }

    #[test]
    fn test_missing_event_id() {
        let mut envelope = valid_envelope();
        envelope.as_object_mut().expect("test").remove("event_id");
        let err = validate_envelope(&envelope).unwrap_err();
        assert_eq!(err, "Missing required field: event_id");
    }

    #[test]
    fn test_invalid_event_id() {
        let mut envelope = valid_envelope();
        envelope["event_id"] = json!("not-a-uuid");
        let err = validate_envelope(&envelope).unwrap_err();
        assert!(err.contains("Invalid event_id"));
    }

    #[test]
    fn test_invalid_occurred_at() {
        let mut envelope = valid_envelope();
        envelope["occurred_at"] = json!("not-a-timestamp");
        let err = validate_envelope(&envelope).unwrap_err();
        assert!(err.contains("Invalid occurred_at"));
    }

    #[test]
    fn test_empty_tenant_id() {
        let mut envelope = valid_envelope();
        envelope["tenant_id"] = json!("   ");
        let err = validate_envelope(&envelope).unwrap_err();
        assert_eq!(err, "Invalid tenant_id: must be non-empty");
    }

    #[test]
    fn test_missing_payload() {
        let mut envelope = valid_envelope();
        envelope.as_object_mut().expect("test").remove("payload");
        let err = validate_envelope(&envelope).unwrap_err();
        assert!(err.contains("payload"));
    }

    #[test]
    fn test_invalid_payload_type() {
        let mut envelope = valid_envelope();
        envelope["payload"] = json!("not-an-object");
        let err = validate_envelope(&envelope).unwrap_err();
        assert!(err.contains("payload"));
    }

    #[test]
    fn test_rejects_missing_mutation_class() {
        let mut envelope = valid_envelope();
        envelope.as_object_mut().expect("test").remove("mutation_class");
        let err = validate_envelope(&envelope).unwrap_err();
        assert!(err.contains("mutation_class"));
    }

    #[test]
    fn test_rejects_invalid_mutation_class() {
        let mut envelope = valid_envelope();
        envelope["mutation_class"] = json!("BOGUS");
        let err = validate_envelope(&envelope).unwrap_err();
        assert!(err.contains("Invalid mutation_class"));
    }

    #[test]
    fn test_accepts_trace_id_alias() {
        let mut envelope = valid_envelope();
        envelope["trace_id"] = json!("trace-123");
        assert!(validate_envelope(&envelope).is_ok());
    }
}

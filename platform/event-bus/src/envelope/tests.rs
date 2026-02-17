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
    assert!(envelope.actor_id.is_none());
    assert!(envelope.actor_type.is_none());
}

#[test]
fn test_envelope_with_builder() {
    let reverses_id = uuid::Uuid::new_v4();
    let supersedes_id = uuid::Uuid::new_v4();

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
fn test_envelope_with_actor() {
    let actor_id = uuid::Uuid::new_v4();

    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "test-module".to_string(),
        "test.event".to_string(),
        json!({"test": "data"}),
    )
    .with_actor(actor_id, "User".to_string());

    assert_eq!(envelope.actor_id, Some(actor_id));
    assert_eq!(envelope.actor_type, Some("User".to_string()));
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

#[test]
fn test_validate_envelope_fields_valid_actor_types() {
    let valid_actor_types = vec!["User", "Service", "System"];

    for actor_type in valid_actor_types {
        let envelope = json!({
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "event_type": "test.event",
            "occurred_at": "2024-01-01T00:00:00Z",
            "tenant_id": "tenant-123",
            "source_module": "test",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "actor_id": "550e8400-e29b-41d4-a716-446655440001",
            "actor_type": actor_type
        });

        assert!(
            validate_envelope_fields(&envelope).is_ok(),
            "Expected {} to be valid",
            actor_type
        );
    }
}

#[test]
fn test_validate_envelope_fields_invalid_actor_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "test.event",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "test",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "actor_id": "550e8400-e29b-41d4-a716-446655440001",
        "actor_type": "InvalidType"
    });

    let result = validate_envelope_fields(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid actor_type"));
}

#[test]
fn test_validate_envelope_fields_empty_actor_type() {
    let envelope = json!({
        "event_id": "550e8400-e29b-41d4-a716-446655440000",
        "event_type": "test.event",
        "occurred_at": "2024-01-01T00:00:00Z",
        "tenant_id": "tenant-123",
        "source_module": "test",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "actor_type": ""
    });

    let result = validate_envelope_fields(&envelope);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("actor_type cannot be empty"));
}

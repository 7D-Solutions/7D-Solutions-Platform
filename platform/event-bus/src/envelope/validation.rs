//! Envelope field validation.
//!
//! Validates serialized event envelopes against platform contract rules.

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

    // Validate actor fields if present (optional for backward compatibility)
    if let Some(actor_type) = envelope.get("actor_type").and_then(|v| v.as_str()) {
        if actor_type.is_empty() {
            return Err("actor_type cannot be empty".to_string());
        }
        // Validate actor_type is a known value
        const VALID_ACTOR_TYPES: &[&str] = &["User", "Service", "System"];
        if !VALID_ACTOR_TYPES.contains(&actor_type) {
            return Err(format!(
                "Invalid actor_type: '{}'. Must be one of: {:?}",
                actor_type, VALID_ACTOR_TYPES
            ));
        }
    }

    // reverses_event_id and supersedes_event_id are optional UUIDs
    Ok(())
}

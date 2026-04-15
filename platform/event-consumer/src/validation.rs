//! Consumer-side envelope and subject validation.
//!
//! [`validate_incoming`] runs before routing and handler dispatch,
//! rejecting malformed envelopes and unsafe NATS subjects.

use event_bus::EventEnvelope;

/// Validation failure — always non-retryable (→ DLQ as Poison).
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("invalid envelope: {0}")]
    Envelope(String),

    #[error("invalid subject: {0}")]
    Subject(String),
}

/// Validate an incoming envelope and its delivery subject.
///
/// Must be called BEFORE routing/handler dispatch. Failures are
/// non-retryable and should be sent to the DLQ.
pub fn validate_incoming(
    envelope: &EventEnvelope<serde_json::Value>,
    subject: &str,
) -> Result<(), ValidationError> {
    validate_envelope(envelope)?;
    validate_subject(subject)?;
    Ok(())
}

fn validate_envelope(envelope: &EventEnvelope<serde_json::Value>) -> Result<(), ValidationError> {
    if envelope.tenant_id.is_empty() {
        return Err(ValidationError::Envelope("tenant_id is empty".into()));
    }
    if envelope.source_module.is_empty() {
        return Err(ValidationError::Envelope("source_module is empty".into()));
    }
    if envelope.event_type.is_empty() {
        return Err(ValidationError::Envelope("event_type is empty".into()));
    }
    if envelope.event_id.is_nil() {
        return Err(ValidationError::Envelope("event_id is nil UUID".into()));
    }
    if !is_semver(&envelope.schema_version) {
        return Err(ValidationError::Envelope(format!(
            "schema_version '{}' is not valid semver",
            envelope.schema_version
        )));
    }
    Ok(())
}

fn validate_subject(subject: &str) -> Result<(), ValidationError> {
    if subject.is_empty() {
        return Err(ValidationError::Subject("subject is empty".into()));
    }
    if subject.contains('*') {
        return Err(ValidationError::Subject(
            "subject contains wildcard '*'".into(),
        ));
    }
    if subject.contains('>') {
        return Err(ValidationError::Subject(
            "subject contains wildcard '>'".into(),
        ));
    }
    if subject.starts_with('.') {
        return Err(ValidationError::Subject("leading dot".into()));
    }
    if subject.ends_with('.') {
        return Err(ValidationError::Subject("trailing dot".into()));
    }
    if subject.contains("..") {
        return Err(ValidationError::Subject(
            "empty segment (consecutive dots)".into(),
        ));
    }
    Ok(())
}

/// Minimal semver check: MAJOR.MINOR.PATCH where each is a non-negative integer.
fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.parse::<u64>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use event_bus::EventEnvelope;

    fn good_envelope() -> EventEnvelope<serde_json::Value> {
        EventEnvelope::new(
            "tenant-1".to_string(),
            "inventory".to_string(),
            "inventory.item_issued".to_string(),
            serde_json::json!({"key": "value"}),
        )
        .with_schema_version("1.0.0".to_string())
    }

    // -- envelope validation --

    #[test]
    fn valid_envelope_and_subject_passes() {
        let env = good_envelope();
        assert!(validate_incoming(&env, "inventory.item_issued").is_ok());
    }

    #[test]
    fn empty_tenant_id_rejected() {
        let mut env = good_envelope();
        env.tenant_id = String::new();
        let err = validate_incoming(&env, "inventory.item_issued").unwrap_err();
        assert!(err.to_string().contains("tenant_id"));
    }

    #[test]
    fn empty_source_module_rejected() {
        let mut env = good_envelope();
        env.source_module = String::new();
        let err = validate_incoming(&env, "inventory.item_issued").unwrap_err();
        assert!(err.to_string().contains("source_module"));
    }

    #[test]
    fn empty_event_type_rejected() {
        let mut env = good_envelope();
        env.event_type = String::new();
        let err = validate_incoming(&env, "inventory.item_issued").unwrap_err();
        assert!(err.to_string().contains("event_type"));
    }

    #[test]
    fn nil_event_id_rejected() {
        let mut env = good_envelope();
        env.event_id = uuid::Uuid::nil();
        let err = validate_incoming(&env, "inventory.item_issued").unwrap_err();
        assert!(err.to_string().contains("event_id"));
    }

    #[test]
    fn bad_schema_version_rejected() {
        let mut env = good_envelope();
        env.schema_version = "not-semver".to_string();
        let err = validate_incoming(&env, "inventory.item_issued").unwrap_err();
        assert!(err.to_string().contains("schema_version"));
    }

    #[test]
    fn schema_version_missing_patch_rejected() {
        let mut env = good_envelope();
        env.schema_version = "1.0".to_string();
        assert!(validate_incoming(&env, "inventory.item_issued").is_err());
    }

    // -- subject validation --

    #[test]
    fn wildcard_star_rejected() {
        let env = good_envelope();
        let err = validate_incoming(&env, "inventory.*").unwrap_err();
        assert!(err.to_string().contains("wildcard '*'"));
    }

    #[test]
    fn wildcard_gt_rejected() {
        let env = good_envelope();
        let err = validate_incoming(&env, "inventory.>").unwrap_err();
        assert!(err.to_string().contains("wildcard '>'"));
    }

    #[test]
    fn empty_subject_rejected() {
        let env = good_envelope();
        let err = validate_incoming(&env, "").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn leading_dot_rejected() {
        let env = good_envelope();
        let err = validate_incoming(&env, ".inventory.item_issued").unwrap_err();
        assert!(err.to_string().contains("leading dot"));
    }

    #[test]
    fn trailing_dot_rejected() {
        let env = good_envelope();
        let err = validate_incoming(&env, "inventory.item_issued.").unwrap_err();
        assert!(err.to_string().contains("trailing dot"));
    }

    #[test]
    fn empty_segment_rejected() {
        let env = good_envelope();
        let err = validate_incoming(&env, "inventory..item_issued").unwrap_err();
        assert!(err.to_string().contains("empty segment"));
    }

    #[test]
    fn star_in_middle_rejected() {
        let env = good_envelope();
        assert!(validate_incoming(&env, "inv*ntory.item").is_err());
    }

    #[test]
    fn gt_in_middle_rejected() {
        let env = good_envelope();
        assert!(validate_incoming(&env, "inventory>item").is_err());
    }
}

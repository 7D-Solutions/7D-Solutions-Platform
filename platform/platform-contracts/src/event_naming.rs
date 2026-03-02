//! # Event Naming & Versioning Conventions
//!
//! ## NATS Subject Format
//!
//! ```text
//! {module}.events.{event_type}
//! ```
//!
//! Example: `ar.events.invoice.created`, `notifications.events.delivery.failed`
//!
//! ## Event Type Format
//!
//! ```text
//! {entity}.{action}[.{qualifier}]
//! ```
//!
//! - Lowercase, dot-delimited.
//! - Singular entity names (`invoice`, not `invoices`).
//! - Past tense for **facts** (`invoice.created`, `payment.succeeded`).
//! - `.requested` suffix for **commands** (`payment.collection.requested`).
//!
//! ## Schema Versioning
//!
//! Each event carries a `schema_version` field (e.g. `"1"`, `"2"`).
//!
//! - **Additive changes** (new optional field): keep the same major version.
//! - **Breaking changes** (remove field, change type, rename): bump major
//!   version *and* create a new JSON Schema file (`{event}-v2.json`).
//! - Producers set `schema_version` to the version they emit.
//! - Consumers must handle older schema versions until an explicit cutover.
//!
//! ## Compatibility Rules
//!
//! 1. Never remove fields from event payloads.
//! 2. Only add fields with safe defaults (consumers that ignore the new
//!    field must still behave correctly).
//! 3. If a breaking change is unavoidable, emit the old AND new event types
//!    during a migration window, then drop the old one.

/// Build a NATS subject from module name and event type.
///
/// ```
/// # use platform_contracts::event_naming::nats_subject;
/// assert_eq!(nats_subject("ar", "invoice.created"), "ar.events.invoice.created");
/// ```
pub fn nats_subject(module: &str, event_type: &str) -> String {
    format!("{}.events.{}", module, event_type)
}

/// Validate that an event type follows the `{entity}.{action}` convention.
///
/// Returns `Ok(())` if the event type has at least two dot-separated segments,
/// all lowercase, and non-empty.
pub fn validate_event_type(event_type: &str) -> Result<(), String> {
    if event_type.is_empty() {
        return Err("event_type cannot be empty".into());
    }

    let segments: Vec<&str> = event_type.split('.').collect();
    if segments.len() < 2 {
        return Err(format!(
            "event_type '{}' must have at least two dot-separated segments (entity.action)",
            event_type,
        ));
    }

    for seg in &segments {
        if seg.is_empty() {
            return Err(format!("event_type '{}' contains an empty segment", event_type));
        }
        if *seg != seg.to_lowercase() {
            return Err(format!(
                "event_type '{}' must be lowercase (segment '{}' is not)",
                event_type, seg,
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nats_subject_format() {
        assert_eq!(nats_subject("ar", "invoice.created"), "ar.events.invoice.created");
        assert_eq!(
            nats_subject("notifications", "delivery.failed"),
            "notifications.events.delivery.failed"
        );
    }

    #[test]
    fn valid_event_types() {
        assert!(validate_event_type("invoice.created").is_ok());
        assert!(validate_event_type("payment.collection.requested").is_ok());
        assert!(validate_event_type("doc.released").is_ok());
    }

    #[test]
    fn invalid_event_types() {
        assert!(validate_event_type("").is_err());
        assert!(validate_event_type("invoice").is_err()); // single segment
        assert!(validate_event_type("Invoice.Created").is_err()); // uppercase
        assert!(validate_event_type(".created").is_err()); // empty segment
        assert!(validate_event_type("invoice.").is_err()); // trailing dot
    }
}

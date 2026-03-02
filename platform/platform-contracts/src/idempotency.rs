//! # Idempotency Key Conventions
//!
//! These rules apply to **all command APIs** across the platform.  Every
//! write endpoint that accepts an `Idempotency-Key` header must follow
//! these conventions.
//!
//! ## Key Format
//!
//! ```text
//! {domain}:{operation}:{tenant_id}:{entity_id}[:{qualifier}]
//! ```
//!
//! - **Deterministic**: same inputs always produce the same key.
//! - **Scoped**: keys are scoped to a tenant — never share keys across tenants.
//! - **Grain-appropriate**: the key boundary matches the operation boundary
//!   (e.g. one key per payment attempt, not one per invoice).
//!
//! Examples:
//! - `payment:attempt:tenant-123:pay_abc:0`
//! - `invoice:create:tenant-123:inv_xyz`
//! - `doc:release:tenant-123:doc_456`
//!
//! ## Storage Pattern
//!
//! ```sql
//! CREATE TABLE {module}_idempotency_keys (
//!     id          SERIAL PRIMARY KEY,
//!     app_id      VARCHAR(255) NOT NULL,
//!     idempotency_key VARCHAR(512) NOT NULL,
//!     request_hash    VARCHAR(64),
//!     response_body   JSONB NOT NULL,
//!     status_code     INT NOT NULL,
//!     created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
//!     expires_at  TIMESTAMP NOT NULL,
//!     UNIQUE (app_id, idempotency_key)
//! );
//! ```
//!
//! ## TTL
//!
//! Default expiry: **24 hours** from creation.  Modules may extend this for
//! long-running operations but must never shorten below 24h.
//!
//! ## Replay Behavior
//!
//! When a duplicate `Idempotency-Key` arrives within the TTL window:
//!
//! 1. Return the **stored response** (status code + body) verbatim.
//! 2. Do **not** re-execute the operation.
//! 3. Log a replay event for observability (`event_type: idempotency.replay`).
//!
//! ## HTTP Header
//!
//! Clients send: `Idempotency-Key: <key>`.  The header is **optional** — if
//! absent, the request is treated as non-idempotent (no replay protection).
//! Write endpoints should document whether they *require* or *accept* the
//! header.

use std::time::Duration;

/// Default idempotency key TTL: 24 hours.
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// HTTP header name for idempotency keys.
pub const HEADER_NAME: &str = "Idempotency-Key";

/// Maximum key length (bytes).  Keys longer than this are rejected.
pub const MAX_KEY_LENGTH: usize = 512;

/// Build a deterministic idempotency key from parts.
///
/// ```
/// # use platform_contracts::idempotency::build_key;
/// let key = build_key("payment", "attempt", "tenant-123", "pay_abc", Some("0"));
/// assert_eq!(key, "payment:attempt:tenant-123:pay_abc:0");
///
/// let key = build_key("invoice", "create", "tenant-123", "inv_xyz", None);
/// assert_eq!(key, "invoice:create:tenant-123:inv_xyz");
/// ```
pub fn build_key(
    domain: &str,
    operation: &str,
    tenant_id: &str,
    entity_id: &str,
    qualifier: Option<&str>,
) -> String {
    match qualifier {
        Some(q) => format!("{}:{}:{}:{}:{}", domain, operation, tenant_id, entity_id, q),
        None => format!("{}:{}:{}:{}", domain, operation, tenant_id, entity_id),
    }
}

/// Validate an idempotency key.
///
/// Rules:
/// - Non-empty
/// - At most [`MAX_KEY_LENGTH`] bytes
/// - At least 3 colon-separated segments (domain:operation:scope)
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("idempotency key cannot be empty".into());
    }
    if key.len() > MAX_KEY_LENGTH {
        return Err(format!(
            "idempotency key exceeds {} byte limit (got {})",
            MAX_KEY_LENGTH,
            key.len()
        ));
    }
    let segments: Vec<&str> = key.split(':').collect();
    if segments.len() < 3 {
        return Err(format!(
            "idempotency key '{}' must have at least 3 colon-separated segments",
            key
        ));
    }
    for seg in &segments {
        if seg.is_empty() {
            return Err(format!("idempotency key '{}' contains an empty segment", key));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_key_with_qualifier() {
        let key = build_key("payment", "attempt", "t1", "p1", Some("0"));
        assert_eq!(key, "payment:attempt:t1:p1:0");
    }

    #[test]
    fn build_key_without_qualifier() {
        let key = build_key("invoice", "create", "t1", "inv1", None);
        assert_eq!(key, "invoice:create:t1:inv1");
    }

    #[test]
    fn validate_good_key() {
        assert!(validate_key("payment:attempt:t1:p1:0").is_ok());
        assert!(validate_key("invoice:create:t1").is_ok());
    }

    #[test]
    fn validate_bad_keys() {
        assert!(validate_key("").is_err());
        assert!(validate_key("ab").is_err()); // too few segments
        assert!(validate_key("a::b").is_err()); // empty segment
        let long = "x".repeat(MAX_KEY_LENGTH + 1);
        assert!(validate_key(&long).is_err());
    }
}

//! PII redaction helpers for safe logging and metrics.
//!
//! Use [`Redacted<T>`] to wrap sensitive values so they never appear in
//! tracing spans, log fields, or error messages. Use the free functions
//! ([`redact_email`], [`redact_partial`]) when you need a partially-masked
//! representation for debugging purposes (e.g. audit trails where some
//! context is required without exposing the full value).
//!
//! # Quick start
//!
//! ```rust
//! use security::redaction::{Redacted, redact_email};
//!
//! let email = "alice@example.com".to_string();
//! // Wrap in Redacted so tracing cannot leak it
//! let safe = Redacted(email.clone());
//! assert_eq!(format!("{safe}"), "[REDACTED]");
//! assert_eq!(format!("{safe:?}"), "[REDACTED]");
//!
//! // Partially masked for human-readable audit context
//! assert_eq!(redact_email(&email), "[redacted]@example.com");
//! ```
//!
//! # PII field inventory
//!
//! The following fields are considered PII across all platform modules.
//! They MUST NOT appear verbatim in log statements or metric labels.
//!
//! | Category        | Fields                                                  |
//! |-----------------|----------------------------------------------------------|
//! | Identity        | `email`, `name`, `phone`, `date_of_birth`               |
//! | Financial       | `card_number`, `account_number`, `routing_number`        |
//! | Tax / Legal     | `ssn`, `tax_id`, `vat_number`, `ein`                    |
//! | Address         | `street`, `city`, `state`, `postal_code`, `country`     |
//! | Credentials     | `password`, `secret`, `api_key`, `token` (full values)  |
//!
//! Safe to log: internal IDs (`customer_id`, `invoice_id`, `tenant_id`),
//! status codes, metric counts, and durations.

use std::fmt;

/// A transparent wrapper that prevents the inner value from appearing in
/// [`Debug`] or [`Display`] output.
///
/// The inner value is still fully accessible via `.0` or [`Redacted::into_inner`].
///
/// ```rust
/// use security::redaction::Redacted;
///
/// let secret = Redacted("hunter2".to_string());
/// assert_eq!(format!("{secret:?}"), "[REDACTED]");
/// assert_eq!(format!("{secret}"), "[REDACTED]");
/// // Access the inner value explicitly when needed
/// assert_eq!(secret.into_inner(), "hunter2");
/// ```
pub struct Redacted<T>(pub T);

impl<T> Redacted<T> {
    /// Consume the wrapper and return the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Borrow the inner value.
    pub fn inner(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T: Clone> Clone for Redacted<T> {
    fn clone(&self) -> Self {
        Redacted(self.0.clone())
    }
}

/// Mask an email address, preserving the domain for audit context.
///
/// `alice@example.com` → `[redacted]@example.com`
///
/// If the input cannot be parsed as an email, returns `"[redacted]"`.
pub fn redact_email(email: &str) -> String {
    match email.split_once('@') {
        Some((_, domain)) if !domain.is_empty() => format!("[redacted]@{domain}"),
        _ => "[redacted]".to_string(),
    }
}

/// Mask all but the last `visible` characters of a string.
///
/// `redact_partial("4111111111111234", 4)` → `"XXXXXXXXXXXX1234"`
///
/// If the string is shorter than or equal to `visible`, returns all `X`s.
pub fn redact_partial(value: &str, visible: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= visible {
        return "X".repeat(chars.len());
    }
    let mask_count = chars.len() - visible;
    let masked = "X".repeat(mask_count);
    let tail: String = chars[mask_count..].iter().collect();
    format!("{masked}{tail}")
}

/// Mask a name, showing only initials.
///
/// `"Alice Bob"` → `"A. B."`
/// Single-word names → `"A."`
pub fn redact_name(name: &str) -> String {
    let initials: Vec<String> = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .map(|c| format!("{c}."))
        .collect();
    if initials.is_empty() {
        "[redacted]".to_string()
    } else {
        initials.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_wrapper_hides_value_in_debug() {
        let r = Redacted("super-secret");
        assert_eq!(format!("{r:?}"), "[REDACTED]");
    }

    #[test]
    fn redaction_wrapper_hides_value_in_display() {
        let r = Redacted(42u64);
        assert_eq!(format!("{r}"), "[REDACTED]");
    }

    #[test]
    fn redaction_wrapper_inner_accessible() {
        let r = Redacted("hunter2".to_string());
        assert_eq!(r.inner(), "hunter2");
        assert_eq!(r.into_inner(), "hunter2");
    }

    #[test]
    fn redaction_wrapper_clone() {
        let r = Redacted("secret".to_string());
        let r2 = r.clone();
        assert_eq!(r.inner(), r2.inner());
    }

    #[test]
    fn redact_email_preserves_domain() {
        assert_eq!(redact_email("alice@example.com"), "[redacted]@example.com");
        assert_eq!(
            redact_email("bob@corp.internal"),
            "[redacted]@corp.internal"
        );
    }

    #[test]
    fn redact_email_invalid_falls_back() {
        assert_eq!(redact_email("notanemail"), "[redacted]");
        assert_eq!(redact_email("missing@"), "[redacted]");
        assert_eq!(redact_email(""), "[redacted]");
    }

    #[test]
    fn redact_partial_masks_prefix() {
        assert_eq!(redact_partial("4111111111111234", 4), "XXXXXXXXXXXX1234");
        assert_eq!(redact_partial("abcde", 2), "XXXde");
    }

    #[test]
    fn redact_partial_short_string_all_masked() {
        assert_eq!(redact_partial("ab", 4), "XX");
        assert_eq!(redact_partial("", 4), "");
    }

    #[test]
    fn redact_name_shows_initials() {
        assert_eq!(redact_name("Alice Bob"), "A. B.");
        assert_eq!(redact_name("Charlie"), "C.");
        assert_eq!(redact_name("Alice Marie Smith"), "A. M. S.");
    }

    #[test]
    fn redact_name_empty_falls_back() {
        assert_eq!(redact_name(""), "[redacted]");
        assert_eq!(redact_name("   "), "[redacted]");
    }

    #[test]
    fn redaction_safe_in_tracing_field() {
        // Simulate what tracing does: call Display on the value
        let email = Redacted("alice@example.com".to_string());
        // Would appear in log as: email = [REDACTED]
        let logged = format!("{email}");
        assert!(
            !logged.contains('@'),
            "email address must not appear in logs"
        );
        assert!(
            !logged.contains("alice"),
            "email local part must not appear in logs"
        );
    }

    #[test]
    fn redact_email_no_pii_in_output() {
        let masked = redact_email("alice@example.com");
        assert!(!masked.contains("alice"), "local part must be redacted");
        assert!(
            masked.contains("example.com"),
            "domain preserved for audit context"
        );
    }
}

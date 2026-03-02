//! # Mutation Classes
//!
//! Every event envelope carries a `mutation_class` that classifies the type
//! of change.  These are the **only** valid values platform-wide.
//!
//! | Class | Meaning | Idempotent? |
//! |-------|---------|-------------|
//! | `DATA_MUTATION` | Financial or audit mutation | Yes |
//! | `REVERSAL` | Compensating transaction | Yes |
//! | `CORRECTION` | Superseding correction | Yes |
//! | `SIDE_EFFECT` | Non-idempotent external action | No |
//! | `QUERY` | Read-only operation | Yes |
//! | `LIFECYCLE` | Entity lifecycle transition | Yes |
//! | `ADMINISTRATIVE` | Configuration / setup change | Yes |
//!
//! Financial modules (`ar`, `gl`, `payments`, `ap`, `treasury`, `billing`,
//! `ttp`) with financial mutation classes (`DATA_MUTATION`, `REVERSAL`,
//! `CORRECTION`) **must** include a `merchant_context` on the envelope.

/// All valid mutation class values.
pub const VALID_CLASSES: &[&str] = &[
    DATA_MUTATION,
    REVERSAL,
    CORRECTION,
    SIDE_EFFECT,
    QUERY,
    LIFECYCLE,
    ADMINISTRATIVE,
];

pub const DATA_MUTATION: &str = "DATA_MUTATION";
pub const REVERSAL: &str = "REVERSAL";
pub const CORRECTION: &str = "CORRECTION";
pub const SIDE_EFFECT: &str = "SIDE_EFFECT";
pub const QUERY: &str = "QUERY";
pub const LIFECYCLE: &str = "LIFECYCLE";
pub const ADMINISTRATIVE: &str = "ADMINISTRATIVE";

/// Financial mutation classes that require `merchant_context`.
pub const FINANCIAL_MUTATION_CLASSES: &[&str] = &[DATA_MUTATION, REVERSAL, CORRECTION];

/// Modules whose financial mutations require `merchant_context`.
pub const FINANCIAL_MODULES: &[&str] = &["ar", "gl", "payments", "ap", "treasury", "billing", "ttp"];

/// Check whether a mutation class value is valid.
pub fn is_valid(class: &str) -> bool {
    VALID_CLASSES.contains(&class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_classes() {
        assert!(is_valid("DATA_MUTATION"));
        assert!(is_valid("LIFECYCLE"));
    }

    #[test]
    fn invalid_classes() {
        assert!(!is_valid("UNKNOWN"));
        assert!(!is_valid(""));
    }
}

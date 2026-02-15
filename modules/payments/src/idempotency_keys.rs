//! Phase 15 Idempotency Key Builders
//!
//! Deterministic idempotency key generation for lifecycle-critical Payments operations.
//! Implements IDEMPOTENCY-KEYS-V1 specification (bd-1p2).
//!
//! **Principles:**
//! 1. Deterministic: Same inputs → same key
//! 2. Grain-appropriate: Keys match operation boundaries
//! 3. Module-local: No cross-module dependencies
//! 4. DB-enforced: UNIQUE constraints prevent duplicates
//!
//! **Usage:**
//! ```rust
//! use payments::idempotency_keys::{generate_payment_attempt_key, PaymentAttemptKey};
//! use uuid::Uuid;
//!
//! let payment_id = Uuid::new_v4();
//! let key = generate_payment_attempt_key("app-demo", payment_id, 0);
//! ```

use std::fmt;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyGenerationError {
    EmptyAppId,
    InvalidAttemptNo(i32),
}

impl fmt::Display for KeyGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAppId => write!(f, "app_id cannot be empty"),
            Self::InvalidAttemptNo(n) => {
                write!(f, "attempt_no must be 0-2 (retry windows), got {}", n)
            }
        }
    }
}

impl std::error::Error for KeyGenerationError {}

// ============================================================================
// Key Types (Newtype Pattern for Type Safety)
// ============================================================================

/// Idempotency key for payment attempts (PSP execution)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaymentAttemptKey(String);

impl PaymentAttemptKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PaymentAttemptKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// Key Builders
// ============================================================================

/// Generate idempotency key for payment attempt (PSP execution)
///
/// **Format:** `payment:attempt:{app_id}:{payment_id}:{attempt_no}`
///
/// **Example:** `payment:attempt:app-demo:550e8400-e29b-41d4-a716-446655440000:1`
///
/// **Guarantees:**
/// - Exactly one PSP call per payment per attempt window
/// - Deterministic (attempt_no matches AR invoice attempt)
/// - Replay-safe
/// - PSP-level deduplication (key sent to PSP as Idempotency-Key header)
///
/// **Retry Windows:**
/// - Attempt 0: Day 0 (invoice due date)
/// - Attempt 1: Day 3 (due date + 3 days)
/// - Attempt 2: Day 7 (due date + 7 days)
///
/// **Validation:**
/// - app_id must not be empty
/// - attempt_no must be 0-2 (three retry windows)
///
/// **Storage:**
/// - `payment_attempts.idempotency_key`
/// - UNIQUE constraint: (app_id, payment_id, attempt_no)
///
/// **PSP Integration:**
/// - Send key to PSP in `Idempotency-Key` HTTP header
/// - PSP performs its own deduplication using this key
/// - If PSP rejects duplicate, treat as successful no-op
///
/// **Reference:** IDEMPOTENCY-KEYS-V1.md § Payment Attempt
pub fn generate_payment_attempt_key(
    app_id: &str,
    payment_id: Uuid,
    attempt_no: i32,
) -> Result<PaymentAttemptKey, KeyGenerationError> {
    // Validation
    if app_id.is_empty() {
        return Err(KeyGenerationError::EmptyAppId);
    }
    if !(0..=2).contains(&attempt_no) {
        return Err(KeyGenerationError::InvalidAttemptNo(attempt_no));
    }

    // Generate key
    let key = format!("payment:attempt:{}:{}:{}", app_id, payment_id, attempt_no);

    Ok(PaymentAttemptKey(key))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_payment_attempt_key_format() {
        let app_id = "app-demo";
        let payment_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let attempt_no = 1;

        let key = generate_payment_attempt_key(app_id, payment_id, attempt_no).unwrap();

        assert_eq!(
            key.as_str(),
            "payment:attempt:app-demo:550e8400-e29b-41d4-a716-446655440000:1"
        );
    }

    #[test]
    fn test_payment_attempt_key_determinism() {
        let app_id = "app-demo";
        let payment_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let attempt_no = 1;

        let key1 = generate_payment_attempt_key(app_id, payment_id, attempt_no).unwrap();
        let key2 = generate_payment_attempt_key(app_id, payment_id, attempt_no).unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_payment_attempt_key_all_retry_windows() {
        let app_id = "app-demo";
        let payment_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let key0 = generate_payment_attempt_key(app_id, payment_id, 0).unwrap();
        let key1 = generate_payment_attempt_key(app_id, payment_id, 1).unwrap();
        let key2 = generate_payment_attempt_key(app_id, payment_id, 2).unwrap();

        assert_eq!(
            key0.as_str(),
            "payment:attempt:app-demo:550e8400-e29b-41d4-a716-446655440000:0"
        );
        assert_eq!(
            key1.as_str(),
            "payment:attempt:app-demo:550e8400-e29b-41d4-a716-446655440000:1"
        );
        assert_eq!(
            key2.as_str(),
            "payment:attempt:app-demo:550e8400-e29b-41d4-a716-446655440000:2"
        );
    }

    #[test]
    fn test_payment_attempt_key_empty_app_id() {
        let payment_id = Uuid::new_v4();
        let result = generate_payment_attempt_key("", payment_id, 1);
        assert_eq!(result, Err(KeyGenerationError::EmptyAppId));
    }

    #[test]
    fn test_payment_attempt_key_invalid_attempt_no_negative() {
        let payment_id = Uuid::new_v4();
        let result = generate_payment_attempt_key("app-demo", payment_id, -1);
        assert_eq!(result, Err(KeyGenerationError::InvalidAttemptNo(-1)));
    }

    #[test]
    fn test_payment_attempt_key_invalid_attempt_no_too_high() {
        let payment_id = Uuid::new_v4();
        let result = generate_payment_attempt_key("app-demo", payment_id, 3);
        assert_eq!(result, Err(KeyGenerationError::InvalidAttemptNo(3)));
    }

    #[test]
    fn test_payment_attempt_key_uniqueness_across_payments() {
        let app_id = "app-demo";
        let attempt_no = 1;

        let payment_id1 = Uuid::new_v4();
        let payment_id2 = Uuid::new_v4();

        let key1 = generate_payment_attempt_key(app_id, payment_id1, attempt_no).unwrap();
        let key2 = generate_payment_attempt_key(app_id, payment_id2, attempt_no).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_payment_attempt_key_uniqueness_across_attempts() {
        let app_id = "app-demo";
        let payment_id = Uuid::new_v4();

        let key0 = generate_payment_attempt_key(app_id, payment_id, 0).unwrap();
        let key1 = generate_payment_attempt_key(app_id, payment_id, 1).unwrap();
        let key2 = generate_payment_attempt_key(app_id, payment_id, 2).unwrap();

        assert_ne!(key0, key1);
        assert_ne!(key1, key2);
        assert_ne!(key0, key2);
    }

    #[test]
    fn test_payment_attempt_key_stable_uuid() {
        // Same UUID should always produce same key
        let app_id = "app-demo";
        let payment_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let attempt_no = 1;

        let key1 = generate_payment_attempt_key(app_id, payment_id, attempt_no).unwrap();
        let key2 = generate_payment_attempt_key(app_id, payment_id, attempt_no).unwrap();

        assert_eq!(key1, key2);
        assert_eq!(
            key1.as_str(),
            "payment:attempt:app-demo:550e8400-e29b-41d4-a716-446655440000:1"
        );
    }
}

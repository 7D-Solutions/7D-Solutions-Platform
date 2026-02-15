//! Phase 15 Idempotency Key Builders
//!
//! Deterministic idempotency key generation for lifecycle-critical AR operations.
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
//! use ar::idempotency_keys::{generate_invoice_attempt_key, InvoiceAttemptKey};
//!
//! let key = generate_invoice_attempt_key("app-demo", 123, 0);
//! assert_eq!(key.as_str(), "invoice:attempt:app-demo:123:0");
//! ```

use chrono::NaiveDate;
use std::fmt;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyGenerationError {
    EmptyAppId,
    InvalidAttemptNo(i32),
    InvalidSubscriptionId(i32),
    InvalidInvoiceId(i32),
}

impl fmt::Display for KeyGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAppId => write!(f, "app_id cannot be empty"),
            Self::InvalidAttemptNo(n) => {
                write!(f, "attempt_no must be 0-2 (retry windows), got {}", n)
            }
            Self::InvalidSubscriptionId(id) => {
                write!(f, "subscription_id must be positive, got {}", id)
            }
            Self::InvalidInvoiceId(id) => write!(f, "invoice_id must be positive, got {}", id),
        }
    }
}

impl std::error::Error for KeyGenerationError {}

// ============================================================================
// Key Types (Newtype Pattern for Type Safety)
// ============================================================================

/// Idempotency key for invoice generation (subscription → invoice)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InvoiceGenerationKey(String);

impl InvoiceGenerationKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InvoiceGenerationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Idempotency key for invoice payment collection attempts
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InvoiceAttemptKey(String);

impl InvoiceAttemptKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InvoiceAttemptKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// Key Builders
// ============================================================================

/// Generate idempotency key for invoice generation
///
/// **Format:** `invoice:gen:{app_id}:{subscription_id}:{cycle_start}:{cycle_end}`
///
/// **Example:** `invoice:gen:app-demo:sub-123:2026-02-01:2026-03-01`
///
/// **Guarantees:**
/// - Exactly one invoice per subscription per billing cycle
/// - Deterministic (cycle boundaries are stable)
/// - Replay-safe
///
/// **Validation:**
/// - app_id must not be empty
/// - subscription_id must be positive
/// - Dates are normalized to ISO 8601 (YYYY-MM-DD)
///
/// **Reference:** IDEMPOTENCY-KEYS-V1.md § Invoice Generation
pub fn generate_invoice_generation_key(
    app_id: &str,
    subscription_id: i32,
    cycle_start: NaiveDate,
    cycle_end: NaiveDate,
) -> Result<InvoiceGenerationKey, KeyGenerationError> {
    // Validation
    if app_id.is_empty() {
        return Err(KeyGenerationError::EmptyAppId);
    }
    if subscription_id <= 0 {
        return Err(KeyGenerationError::InvalidSubscriptionId(subscription_id));
    }

    // Generate key
    let key = format!(
        "invoice:gen:{}:{}:{}:{}",
        app_id,
        subscription_id,
        cycle_start.format("%Y-%m-%d"),
        cycle_end.format("%Y-%m-%d")
    );

    Ok(InvoiceGenerationKey(key))
}

/// Generate idempotency key for invoice payment collection attempt
///
/// **Format:** `invoice:attempt:{app_id}:{invoice_id}:{attempt_no}`
///
/// **Example:** `invoice:attempt:app-demo:inv-456:1`
///
/// **Guarantees:**
/// - Exactly one attempt per invoice per retry window
/// - Deterministic (attempt_no is 0, 1, 2)
/// - Replay-safe
///
/// **Retry Windows:**
/// - Attempt 0: Day 0 (invoice due date)
/// - Attempt 1: Day 3 (due date + 3 days)
/// - Attempt 2: Day 7 (due date + 7 days)
///
/// **Validation:**
/// - app_id must not be empty
/// - invoice_id must be positive
/// - attempt_no must be 0-2 (three retry windows)
///
/// **Storage:**
/// - `ar_invoice_attempts.idempotency_key`
/// - UNIQUE constraint: (app_id, invoice_id, attempt_no)
///
/// **Reference:** IDEMPOTENCY-KEYS-V1.md § Invoice Attempt
pub fn generate_invoice_attempt_key(
    app_id: &str,
    invoice_id: i32,
    attempt_no: i32,
) -> Result<InvoiceAttemptKey, KeyGenerationError> {
    // Validation
    if app_id.is_empty() {
        return Err(KeyGenerationError::EmptyAppId);
    }
    if invoice_id <= 0 {
        return Err(KeyGenerationError::InvalidInvoiceId(invoice_id));
    }
    if !(0..=2).contains(&attempt_no) {
        return Err(KeyGenerationError::InvalidAttemptNo(attempt_no));
    }

    // Generate key
    let key = format!("invoice:attempt:{}:{}:{}", app_id, invoice_id, attempt_no);

    Ok(InvoiceAttemptKey(key))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_invoice_generation_key_format() {
        let app_id = "app-demo";
        let subscription_id = 123;
        let cycle_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let cycle_end = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

        let key = generate_invoice_generation_key(app_id, subscription_id, cycle_start, cycle_end)
            .unwrap();

        assert_eq!(
            key.as_str(),
            "invoice:gen:app-demo:123:2026-02-01:2026-03-01"
        );
    }

    #[test]
    fn test_invoice_generation_key_determinism() {
        let app_id = "app-demo";
        let subscription_id = 123;
        let cycle_start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let cycle_end = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

        let key1 =
            generate_invoice_generation_key(app_id, subscription_id, cycle_start, cycle_end)
                .unwrap();
        let key2 =
            generate_invoice_generation_key(app_id, subscription_id, cycle_start, cycle_end)
                .unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_invoice_generation_key_empty_app_id() {
        let result = generate_invoice_generation_key(
            "",
            123,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        );

        assert_eq!(result, Err(KeyGenerationError::EmptyAppId));
    }

    #[test]
    fn test_invoice_generation_key_invalid_subscription_id() {
        let result = generate_invoice_generation_key(
            "app-demo",
            0,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        );

        assert_eq!(result, Err(KeyGenerationError::InvalidSubscriptionId(0)));
    }

    #[test]
    fn test_invoice_attempt_key_format() {
        let app_id = "app-demo";
        let invoice_id = 456;
        let attempt_no = 1;

        let key = generate_invoice_attempt_key(app_id, invoice_id, attempt_no).unwrap();

        assert_eq!(key.as_str(), "invoice:attempt:app-demo:456:1");
    }

    #[test]
    fn test_invoice_attempt_key_determinism() {
        let app_id = "app-demo";
        let invoice_id = 456;
        let attempt_no = 1;

        let key1 = generate_invoice_attempt_key(app_id, invoice_id, attempt_no).unwrap();
        let key2 = generate_invoice_attempt_key(app_id, invoice_id, attempt_no).unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_invoice_attempt_key_all_retry_windows() {
        let app_id = "app-demo";
        let invoice_id = 456;

        let key0 = generate_invoice_attempt_key(app_id, invoice_id, 0).unwrap();
        let key1 = generate_invoice_attempt_key(app_id, invoice_id, 1).unwrap();
        let key2 = generate_invoice_attempt_key(app_id, invoice_id, 2).unwrap();

        assert_eq!(key0.as_str(), "invoice:attempt:app-demo:456:0");
        assert_eq!(key1.as_str(), "invoice:attempt:app-demo:456:1");
        assert_eq!(key2.as_str(), "invoice:attempt:app-demo:456:2");
    }

    #[test]
    fn test_invoice_attempt_key_empty_app_id() {
        let result = generate_invoice_attempt_key("", 456, 1);
        assert_eq!(result, Err(KeyGenerationError::EmptyAppId));
    }

    #[test]
    fn test_invoice_attempt_key_invalid_invoice_id() {
        let result = generate_invoice_attempt_key("app-demo", 0, 1);
        assert_eq!(result, Err(KeyGenerationError::InvalidInvoiceId(0)));
    }

    #[test]
    fn test_invoice_attempt_key_invalid_attempt_no_negative() {
        let result = generate_invoice_attempt_key("app-demo", 456, -1);
        assert_eq!(result, Err(KeyGenerationError::InvalidAttemptNo(-1)));
    }

    #[test]
    fn test_invoice_attempt_key_invalid_attempt_no_too_high() {
        let result = generate_invoice_attempt_key("app-demo", 456, 3);
        assert_eq!(result, Err(KeyGenerationError::InvalidAttemptNo(3)));
    }

    #[test]
    fn test_invoice_attempt_key_uniqueness_across_invoices() {
        let app_id = "app-demo";
        let attempt_no = 1;

        let key1 = generate_invoice_attempt_key(app_id, 100, attempt_no).unwrap();
        let key2 = generate_invoice_attempt_key(app_id, 200, attempt_no).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_invoice_attempt_key_uniqueness_across_attempts() {
        let app_id = "app-demo";
        let invoice_id = 456;

        let key0 = generate_invoice_attempt_key(app_id, invoice_id, 0).unwrap();
        let key1 = generate_invoice_attempt_key(app_id, invoice_id, 1).unwrap();
        let key2 = generate_invoice_attempt_key(app_id, invoice_id, 2).unwrap();

        assert_ne!(key0, key1);
        assert_ne!(key1, key2);
        assert_ne!(key0, key2);
    }
}

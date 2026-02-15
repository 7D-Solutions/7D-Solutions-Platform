//! Webhook Signature Validation Module (Phase 15 - bd-1wg)
//!
//! **CRITICAL: Signature verification BEFORE any database writes.**
//!
//! **Current Implementation:** Stub for internal events (always returns Ok)
//! **Future (PSP Integration):** Stripe HMAC, Tilled signature verification
//!
//! **Mutation Order Enforcement:**
//! 1. Signature validation (this module) ← FIRST
//! 2. Envelope validation (existing module)
//! 3. Attempt ledger gating (SELECT FOR UPDATE)
//! 4. Lifecycle mutation (bd-3lm guards)
//! 5. Event emission
//!
//! **PSP-Specific Implementation (TODO in future beads):**
//! - Stripe: HMAC-SHA256 verification with webhook secret
//! - Tilled: Custom signature verification
//! - Braintree: Custom signature verification
//!
//! **Design Pattern:**
//! - validate_webhook_signature() returns Result immediately
//! - Zero database I/O during signature validation
//! - Reject invalid signatures before any side effects

use std::fmt;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// Signature verification failed (invalid signature)
    InvalidSignature { reason: String },
    /// Missing required signature headers
    MissingSignature,
    /// Unsupported webhook source
    UnsupportedSource { source: String },
}

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSignature { reason } => {
                write!(f, "Webhook signature verification failed: {}", reason)
            }
            Self::MissingSignature => write!(f, "Missing required webhook signature headers"),
            Self::UnsupportedSource { source } => {
                write!(f, "Unsupported webhook source: {}", source)
            }
        }
    }
}

impl std::error::Error for SignatureError {}

// ============================================================================
// Webhook Source Types
// ============================================================================

/// Webhook source (internal events vs external PSP webhooks)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookSource {
    /// Internal event (ar.payment.collection.requested)
    Internal,
    /// Stripe webhook callback (future - bd-XXX)
    Stripe,
    /// Tilled webhook callback (future - bd-XXX)
    Tilled,
}

// ============================================================================
// Signature Validation
// ============================================================================

/// Validate webhook signature BEFORE any database writes
///
/// **CRITICAL INVARIANT:** This function MUST be called FIRST in webhook processing.
/// - NO database writes before signature validation
/// - NO envelope validation before signature validation
/// - NO attempt ledger mutations before signature validation
/// - Returns Result<(), SignatureError> ONLY
///
/// **Current Implementation (Internal Events Only):**
/// - Internal events: Always returns Ok (no signature validation needed)
/// - PSP webhooks: Returns Err(UnsupportedSource) with TODO marker
///
/// **Future Implementation (PSP Webhooks - bd-XXX):**
/// ```ignore
/// match source {
///     WebhookSource::Stripe => {
///         // Stripe HMAC-SHA256 verification
///         // 1. Extract signature from Stripe-Signature header
///         // 2. Compute HMAC-SHA256(webhook_secret, raw_body)
///         // 3. Compare signatures (constant-time)
///         // 4. Verify timestamp to prevent replay attacks
///     }
///     WebhookSource::Tilled => {
///         // Tilled signature verification
///         // (PSP-specific implementation)
///     }
///     WebhookSource::Internal => Ok(())
/// }
/// ```
///
/// **Example Usage:**
/// ```ignore
/// use payments::webhook_signature::{validate_webhook_signature, WebhookSource};
///
/// // Step 1: Signature validation (FIRST - before any DB writes)
/// validate_webhook_signature(WebhookSource::Internal, &headers, &body)?;
///
/// // Step 2: Envelope validation
/// // Step 3: Attempt ledger gating
/// // Step 4: Lifecycle mutation
/// // Step 5: Event emission
/// ```
///
/// **Security Properties:**
/// - Prevents replay attacks (Stripe: timestamp validation)
/// - Prevents tampering (HMAC verification)
/// - Constant-time comparison (prevents timing attacks)
/// - Zero side effects on failure (no database writes)
pub fn validate_webhook_signature(
    source: WebhookSource,
    _headers: &std::collections::HashMap<String, String>,
    _body: &[u8],
) -> Result<(), SignatureError> {
    match source {
        WebhookSource::Internal => {
            // Internal events do not require signature validation
            // (event envelope validation happens in step 2)
            Ok(())
        }
        WebhookSource::Stripe => {
            // TODO (bd-XXX): Implement Stripe HMAC-SHA256 verification
            // 1. Extract Stripe-Signature header (format: "t=timestamp,v1=signature")
            // 2. Parse timestamp and signature components
            // 3. Construct signed payload: format!("{}.{}", timestamp, String::from_utf8_lossy(body))
            // 4. Compute HMAC-SHA256 with webhook secret: hmac_sha256(secret, signed_payload)
            // 5. Constant-time compare computed signature with header signature
            // 6. Verify timestamp is within tolerance (prevent replay attacks)
            // 7. Return Ok if valid, Err(InvalidSignature) if not
            //
            // Example implementation:
            // let signature_header = headers.get("stripe-signature")
            //     .ok_or(SignatureError::MissingSignature)?;
            // let (timestamp, signature) = parse_stripe_signature(signature_header)?;
            // let computed = compute_stripe_signature(webhook_secret, timestamp, body)?;
            // if !constant_time_compare(&computed, &signature) {
            //     return Err(SignatureError::InvalidSignature {
            //         reason: "HMAC verification failed".to_string()
            //     });
            // }
            // if !is_timestamp_valid(timestamp, TOLERANCE_SECONDS) {
            //     return Err(SignatureError::InvalidSignature {
            //         reason: "Timestamp outside tolerance window".to_string()
            //     });
            // }
            // Ok(())

            Err(SignatureError::UnsupportedSource {
                source: "stripe".to_string(),
            })
        }
        WebhookSource::Tilled => {
            // TODO (bd-XXX): Implement Tilled signature verification
            // (PSP-specific verification logic - consult Tilled API docs)

            Err(SignatureError::UnsupportedSource {
                source: "tilled".to_string(),
            })
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_internal_webhook_always_valid() {
        let headers = HashMap::new();
        let body = b"{}";

        let result = validate_webhook_signature(WebhookSource::Internal, &headers, body);
        assert!(result.is_ok());
    }

    #[test]
    fn test_stripe_webhook_unsupported() {
        let headers = HashMap::new();
        let body = b"{}";

        let result = validate_webhook_signature(WebhookSource::Stripe, &headers, body);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            SignatureError::UnsupportedSource {
                source: "stripe".to_string()
            }
        );
    }

    #[test]
    fn test_tilled_webhook_unsupported() {
        let headers = HashMap::new();
        let body = b"{}";

        let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            SignatureError::UnsupportedSource {
                source: "tilled".to_string()
            }
        );
    }

    #[test]
    fn test_signature_error_display() {
        let err = SignatureError::InvalidSignature {
            reason: "HMAC mismatch".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Webhook signature verification failed: HMAC mismatch"
        );

        let err = SignatureError::MissingSignature;
        assert_eq!(err.to_string(), "Missing required webhook signature headers");

        let err = SignatureError::UnsupportedSource {
            source: "stripe".to_string(),
        };
        assert_eq!(err.to_string(), "Unsupported webhook source: stripe");
    }
}

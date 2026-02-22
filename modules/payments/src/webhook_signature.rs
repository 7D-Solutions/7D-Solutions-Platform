//! Webhook Signature Validation Module
//!
//! **CRITICAL: Signature verification BEFORE any database writes.**
//!
//! **Mutation Order Enforcement:**
//! 1. Signature validation (this module) ← FIRST
//! 2. Envelope validation (existing module)
//! 3. Attempt ledger gating (SELECT FOR UPDATE)
//! 4. Lifecycle mutation (bd-3lm guards)
//! 5. Event emission
//!
//! **Tilled Signature Format:**
//! The `tilled-signature` header has the form `t=<unix_ts>,v1=<hex_sig>`.
//! The HMAC-SHA256 is computed over `"<timestamp>.<raw_body>"`.
//! Timestamp must be within 5 minutes to prevent replay attacks.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fmt;

type HmacSha256 = Hmac<Sha256>;

/// Replay-window tolerance for Tilled webhook timestamps (5 minutes).
const TILLED_TIMESTAMP_TOLERANCE_SECS: i64 = 300;

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

/// Validate webhook signature BEFORE any database writes.
///
/// **CRITICAL INVARIANT:** Must be called first — no DB I/O before this.
///
/// - `Internal` events always pass (no external signature needed).
/// - `Tilled` events require at least one secret in `tilled_secrets`.
///   Pass two secrets during zero-downtime rotation (old + new both valid).
/// - `Stripe` is not yet implemented.
///
/// Tilled signature format: `tilled-signature: t=<unix_ts>,v1=<hex_sig>`
/// HMAC-SHA256 is over `"<timestamp>.<raw_body>"`.
pub fn validate_webhook_signature(
    source: WebhookSource,
    headers: &std::collections::HashMap<String, String>,
    body: &[u8],
    tilled_secrets: &[&str],
) -> Result<(), SignatureError> {
    match source {
        WebhookSource::Internal => Ok(()),
        WebhookSource::Stripe => Err(SignatureError::UnsupportedSource {
            source: "stripe".to_string(),
        }),
        WebhookSource::Tilled => {
            if tilled_secrets.is_empty() {
                return Err(SignatureError::InvalidSignature {
                    reason: "Tilled webhook secret not configured".to_string(),
                });
            }
            // Try each secret; succeed on the first match (supports rotation overlap).
            let mut last_err = SignatureError::MissingSignature;
            for secret in tilled_secrets {
                match verify_tilled_signature(body, headers, secret) {
                    Ok(()) => return Ok(()),
                    Err(e) => last_err = e,
                }
            }
            Err(last_err)
        }
    }
}

/// Verify Tilled HMAC-SHA256 webhook signature.
///
/// Header format: `tilled-signature: t=<unix_ts>,v1=<hex_sig>`
/// Signed payload: `"<timestamp>.<raw_body>"`
fn verify_tilled_signature(
    body: &[u8],
    headers: &std::collections::HashMap<String, String>,
    secret: &str,
) -> Result<(), SignatureError> {
    // Header lookup is case-insensitive in HTTP — normalise to lowercase.
    let sig_header = headers
        .get("tilled-signature")
        .or_else(|| headers.get("Tilled-Signature"))
        .map(String::as_str)
        .ok_or(SignatureError::MissingSignature)?;

    let mut timestamp = "";
    let mut sig_value = "";
    for part in sig_header.split(',') {
        if let Some(v) = part.strip_prefix("t=") {
            timestamp = v;
        } else if let Some(v) = part.strip_prefix("v1=") {
            sig_value = v;
        }
    }

    if timestamp.is_empty() || sig_value.is_empty() {
        return Err(SignatureError::InvalidSignature {
            reason: "Malformed tilled-signature header (expected t=...,v1=...)".to_string(),
        });
    }

    // Replay-window check.
    let ts_unix: i64 = timestamp.parse().map_err(|_| SignatureError::InvalidSignature {
        reason: "Invalid timestamp in tilled-signature header".to_string(),
    })?;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = now_unix - ts_unix;
    if age.abs() > TILLED_TIMESTAMP_TOLERANCE_SECS {
        return Err(SignatureError::InvalidSignature {
            reason: format!(
                "Webhook timestamp outside replay window (age={}s, tolerance={}s)",
                age, TILLED_TIMESTAMP_TOLERANCE_SECS
            ),
        });
    }

    // Compute expected HMAC: HMAC-SHA256("<timestamp>.<body>")
    let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(body));
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| {
        SignatureError::InvalidSignature {
            reason: "Invalid Tilled webhook secret".to_string(),
        }
    })?;
    mac.update(signed_payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    if expected != sig_value {
        return Err(SignatureError::InvalidSignature {
            reason: "HMAC-SHA256 signature mismatch".to_string(),
        });
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_tilled_sig(secret: &str, ts: i64, body: &[u8]) -> String {
        let signed = format!("{}.{}", ts, String::from_utf8_lossy(body));
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        format!("t={},v1={}", ts, sig)
    }

    fn now_ts() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn test_internal_webhook_always_valid() {
        let headers = HashMap::new();
        let body = b"{}";
        assert!(validate_webhook_signature(WebhookSource::Internal, &headers, body, &[]).is_ok());
    }

    #[test]
    fn test_stripe_webhook_unsupported() {
        let headers = HashMap::new();
        let body = b"{}";
        let result = validate_webhook_signature(WebhookSource::Stripe, &headers, body, &[]);
        assert_eq!(
            result.unwrap_err(),
            SignatureError::UnsupportedSource { source: "stripe".to_string() }
        );
    }

    #[test]
    fn test_tilled_missing_header() {
        let headers = HashMap::new();
        let body = b"{}";
        let result = validate_webhook_signature(
            WebhookSource::Tilled, &headers, body, &["secret"],
        );
        assert_eq!(result.unwrap_err(), SignatureError::MissingSignature);
    }

    #[test]
    fn test_tilled_valid_signature() {
        let secret = "whsec_test_1234";
        let body = b"{\"type\":\"payment_intent.succeeded\"}";
        let ts = now_ts();
        let sig_header = make_tilled_sig(secret, ts, body);
        let mut headers = HashMap::new();
        headers.insert("tilled-signature".to_string(), sig_header);
        assert!(
            validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[secret]).is_ok()
        );
    }

    #[test]
    fn test_tilled_invalid_signature() {
        let secret = "whsec_test_1234";
        let body = b"{\"type\":\"payment_intent.succeeded\"}";
        let ts = now_ts();
        let sig_header = format!("t={},v1=badbadbadbad", ts);
        let mut headers = HashMap::new();
        headers.insert("tilled-signature".to_string(), sig_header);
        let result =
            validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[secret]);
        assert!(matches!(result.unwrap_err(), SignatureError::InvalidSignature { .. }));
    }

    #[test]
    fn test_tilled_replay_rejected() {
        let secret = "whsec_test_1234";
        let body = b"{}";
        // Timestamp 10 minutes in the past
        let ts = now_ts() - 601;
        let sig_header = make_tilled_sig(secret, ts, body);
        let mut headers = HashMap::new();
        headers.insert("tilled-signature".to_string(), sig_header);
        let result =
            validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[secret]);
        assert!(matches!(result.unwrap_err(), SignatureError::InvalidSignature { reason } if reason.contains("replay window")));
    }

    #[test]
    fn test_tilled_no_secret_configured() {
        let body = b"{}";
        let mut headers = HashMap::new();
        headers.insert("tilled-signature".to_string(), "t=123,v1=abc".to_string());
        let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[]);
        assert!(matches!(result.unwrap_err(), SignatureError::InvalidSignature { .. }));
    }

    #[test]
    fn test_tilled_rotation_overlap_accepts_prev_secret() {
        let new_secret = "whsec_new_secret";
        let old_secret = "whsec_old_secret";
        let body = b"{\"type\":\"payment_intent.succeeded\"}";
        let ts = now_ts();
        // Webhook arrives signed with the OLD secret (rotation in progress)
        let sig_header = make_tilled_sig(old_secret, ts, body);
        let mut headers = HashMap::new();
        headers.insert("tilled-signature".to_string(), sig_header);
        // Both secrets in the slice — should pass via old_secret fallback
        assert!(
            validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[new_secret, old_secret]).is_ok()
        );
    }

    #[test]
    fn test_tilled_rotation_overlap_rejects_unknown_secret() {
        let new_secret = "whsec_new_secret";
        let old_secret = "whsec_old_secret";
        let body = b"{}";
        let ts = now_ts();
        let sig_header = make_tilled_sig("whsec_unknown", ts, body);
        let mut headers = HashMap::new();
        headers.insert("tilled-signature".to_string(), sig_header);
        // Neither new nor old match
        let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[new_secret, old_secret]);
        assert!(matches!(result.unwrap_err(), SignatureError::InvalidSignature { .. }));
    }

    #[test]
    fn test_signature_error_display() {
        let err = SignatureError::InvalidSignature { reason: "HMAC mismatch".to_string() };
        assert_eq!(err.to_string(), "Webhook signature verification failed: HMAC mismatch");

        let err = SignatureError::MissingSignature;
        assert_eq!(err.to_string(), "Missing required webhook signature headers");

        let err = SignatureError::UnsupportedSource { source: "stripe".to_string() };
        assert_eq!(err.to_string(), "Unsupported webhook source: stripe");
    }
}

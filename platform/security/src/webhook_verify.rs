//! Webhook signature verification adapters.
//!
//! Provides a `WebhookVerifier` trait and concrete implementations for
//! verifying inbound webhook signatures from external systems.
//!
//! ## Security Properties
//! - Signature verification is **stateless** — no database I/O.
//! - All comparisons use constant-time equality to prevent timing attacks.
//! - Verification happens BEFORE any payload parsing or DB writes.
//!
//! ## Implementations
//! - `StripeVerifier`: HMAC-SHA256 using the `Stripe-Signature` header format.
//! - `GenericHmacVerifier`: Simple HMAC-SHA256 with a custom header name.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Maximum allowed age of a Stripe webhook timestamp (300 seconds).
pub const STRIPE_TIMESTAMP_TOLERANCE_SECS: u64 = 300;

// ============================================================================
// Error Type
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerifyError {
    #[error("Missing required signature header: {header}")]
    MissingHeader { header: String },

    #[error("Malformed signature header")]
    MalformedHeader,

    #[error("Signature mismatch — payload may be tampered")]
    SignatureMismatch,

    #[error("Timestamp outside tolerance window (replay attack prevention)")]
    TimestampExpired,

    #[error("Invalid secret key")]
    InvalidSecret,
}

// ============================================================================
// Trait
// ============================================================================

/// Adapter interface for webhook signature verification.
///
/// Implementors verify that the raw payload matches the signature provided
/// by an external system via HTTP headers.
///
/// # Contract
/// - Pure function: no side effects, no I/O.
/// - Returns `Ok(())` on success, `Err(VerifyError)` on failure.
/// - MUST use constant-time comparison to prevent timing attacks.
pub trait WebhookVerifier: Send + Sync {
    /// Verify the signature of an inbound webhook payload.
    ///
    /// # Arguments
    /// * `headers` — HTTP request headers (lowercase names).
    /// * `raw_body` — Raw request body bytes (pre-JSON-parse).
    fn verify(&self, headers: &HashMap<String, String>, raw_body: &[u8])
        -> Result<(), VerifyError>;
}

// ============================================================================
// Stripe HMAC-SHA256 Verifier
// ============================================================================

/// Verifies Stripe webhook signatures.
///
/// Stripe sends a `Stripe-Signature` header with the format:
/// `t=<unix_timestamp>,v1=<hmac_sha256_hex>`
///
/// Verification:
/// 1. Parse timestamp (`t`) and signature (`v1`) from header.
/// 2. Construct signed payload: `"<timestamp>.<raw_body>"`.
/// 3. Compute HMAC-SHA256 of signed payload using the webhook secret.
/// 4. Compare (constant-time) with the `v1` signature.
/// 5. Verify timestamp is within `STRIPE_TIMESTAMP_TOLERANCE_SECS`.
pub struct StripeVerifier {
    secret: Vec<u8>,
}

impl StripeVerifier {
    /// Create a new `StripeVerifier` with the webhook signing secret.
    pub fn new(secret: &str) -> Self {
        Self {
            secret: secret.as_bytes().to_vec(),
        }
    }
}

impl WebhookVerifier for StripeVerifier {
    fn verify(
        &self,
        headers: &HashMap<String, String>,
        raw_body: &[u8],
    ) -> Result<(), VerifyError> {
        let sig_header = headers
            .get("stripe-signature")
            .ok_or(VerifyError::MissingHeader {
                header: "stripe-signature".to_string(),
            })?;

        let (timestamp_str, v1_sig_hex) = parse_stripe_signature(sig_header)?;

        // Verify timestamp is within tolerance
        let timestamp: u64 = timestamp_str
            .parse()
            .map_err(|_| VerifyError::MalformedHeader)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now.saturating_sub(timestamp) > STRIPE_TIMESTAMP_TOLERANCE_SECS {
            return Err(VerifyError::TimestampExpired);
        }

        // Construct signed payload: "{timestamp}.{raw_body}"
        let signed_payload = {
            let mut buf = timestamp_str.as_bytes().to_vec();
            buf.push(b'.');
            buf.extend_from_slice(raw_body);
            buf
        };

        // Compute HMAC-SHA256
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| VerifyError::InvalidSecret)?;
        mac.update(&signed_payload);
        let computed = mac.finalize().into_bytes();

        // Decode the provided signature
        let provided = decode_hex(v1_sig_hex)?;

        // Constant-time comparison
        if computed.len() != provided.len() || !constant_time_eq(&computed, &provided) {
            return Err(VerifyError::SignatureMismatch);
        }

        Ok(())
    }
}

/// Parse `t=<timestamp>,v1=<hex_sig>` from a Stripe-Signature header.
fn parse_stripe_signature(header: &str) -> Result<(&str, &str), VerifyError> {
    let mut timestamp = None;
    let mut v1 = None;

    for part in header.split(',') {
        if let Some(ts) = part.strip_prefix("t=") {
            timestamp = Some(ts);
        } else if let Some(sig) = part.strip_prefix("v1=") {
            v1 = Some(sig);
        }
    }

    match (timestamp, v1) {
        (Some(t), Some(s)) => Ok((t, s)),
        _ => Err(VerifyError::MalformedHeader),
    }
}

// ============================================================================
// Generic HMAC-SHA256 Verifier
// ============================================================================

/// Verifies webhooks signed with a simple HMAC-SHA256 over the raw body.
///
/// The signature is read from a configurable header and expected to be a
/// hex-encoded HMAC-SHA256 digest of the raw body.
pub struct GenericHmacVerifier {
    secret: Vec<u8>,
    /// Header name to read the signature from (lowercase).
    header_name: String,
    /// Optional prefix to strip from the header value (e.g. "sha256=").
    prefix: Option<String>,
}

impl GenericHmacVerifier {
    pub fn new(secret: &str, header_name: &str, prefix: Option<&str>) -> Self {
        Self {
            secret: secret.as_bytes().to_vec(),
            header_name: header_name.to_lowercase(),
            prefix: prefix.map(str::to_string),
        }
    }
}

impl WebhookVerifier for GenericHmacVerifier {
    fn verify(
        &self,
        headers: &HashMap<String, String>,
        raw_body: &[u8],
    ) -> Result<(), VerifyError> {
        let raw_value = headers
            .get(&self.header_name)
            .ok_or(VerifyError::MissingHeader {
                header: self.header_name.clone(),
            })?;

        let hex_sig = if let Some(prefix) = &self.prefix {
            raw_value
                .strip_prefix(prefix.as_str())
                .ok_or(VerifyError::MalformedHeader)?
        } else {
            raw_value.as_str()
        };

        let provided = decode_hex(hex_sig)?;

        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| VerifyError::InvalidSecret)?;
        mac.update(raw_body);
        let computed = mac.finalize().into_bytes();

        if computed.len() != provided.len() || !constant_time_eq(&computed, &provided) {
            return Err(VerifyError::SignatureMismatch);
        }

        Ok(())
    }
}

// ============================================================================
// No-Op Verifier (for systems without signature headers)
// ============================================================================

/// Pass-through verifier for systems that don't supply a signature.
///
/// Use only when the source system is trusted via network-level controls
/// (e.g., IP allowlist). Never use in production without compensating controls.
pub struct NoopVerifier;

impl WebhookVerifier for NoopVerifier {
    fn verify(
        &self,
        _headers: &HashMap<String, String>,
        _raw_body: &[u8],
    ) -> Result<(), VerifyError> {
        Ok(())
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn decode_hex(s: &str) -> Result<Vec<u8>, VerifyError> {
    if !s.len().is_multiple_of(2) {
        return Err(VerifyError::MalformedHeader);
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| VerifyError::MalformedHeader))
        .collect()
}

/// Constant-time byte slice comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    fn hmac_sha256_hex(secret: &str, message: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(message);
        let result = mac.finalize().into_bytes();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn test_stripe_verifier_valid() {
        let secret = "whsec_test_secret";
        let verifier = StripeVerifier::new(secret);
        let body = b"{\"event\":\"test\"}";

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let timestamp = now.to_string();
        let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(body));
        let sig = hmac_sha256_hex(secret, signed_payload.as_bytes());

        let mut headers = HashMap::new();
        headers.insert(
            "stripe-signature".to_string(),
            format!("t={},v1={}", timestamp, sig),
        );

        assert!(verifier.verify(&headers, body).is_ok());
    }

    #[test]
    fn test_stripe_verifier_bad_signature() {
        let secret = "whsec_test_secret";
        let verifier = StripeVerifier::new(secret);
        let body = b"{}";
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut headers = HashMap::new();
        headers.insert(
            "stripe-signature".to_string(),
            format!(
                "t={},v1={}",
                now, "deadbeef00000000000000000000000000000000000000000000000000000000"
            ),
        );
        assert_eq!(
            verifier.verify(&headers, body),
            Err(VerifyError::SignatureMismatch)
        );
    }

    #[test]
    fn test_stripe_verifier_missing_header() {
        let verifier = StripeVerifier::new("secret");
        let result = verifier.verify(&HashMap::new(), b"{}");
        assert!(matches!(result, Err(VerifyError::MissingHeader { .. })));
    }

    #[test]
    fn test_stripe_verifier_expired_timestamp() {
        let secret = "secret";
        let verifier = StripeVerifier::new(secret);
        let old_timestamp = "1000000"; // Unix epoch + ~11 days — definitely expired
        let signed_payload = format!("{}.{}", old_timestamp, "{}");
        let sig = hmac_sha256_hex(secret, signed_payload.as_bytes());

        let mut headers = HashMap::new();
        headers.insert(
            "stripe-signature".to_string(),
            format!("t={},v1={}", old_timestamp, sig),
        );
        assert_eq!(
            verifier.verify(&headers, b"{}"),
            Err(VerifyError::TimestampExpired)
        );
    }

    #[test]
    fn test_generic_hmac_verifier_valid() {
        let secret = "mysecret";
        let body = b"hello world";
        let sig = hmac_sha256_hex(secret, body);
        let verifier = GenericHmacVerifier::new(secret, "x-hub-signature-256", Some("sha256="));

        let mut headers = HashMap::new();
        headers.insert("x-hub-signature-256".to_string(), format!("sha256={}", sig));

        assert!(verifier.verify(&headers, body).is_ok());
    }

    #[test]
    fn test_generic_hmac_verifier_bad_sig() {
        let verifier = GenericHmacVerifier::new("secret", "x-signature", None);
        let mut headers = HashMap::new();
        headers.insert(
            "x-signature".to_string(),
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        );
        assert_eq!(
            verifier.verify(&headers, b"body"),
            Err(VerifyError::SignatureMismatch)
        );
    }

    #[test]
    fn test_noop_verifier_always_ok() {
        assert!(NoopVerifier.verify(&HashMap::new(), b"anything").is_ok());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hi", b"hello"));
    }
}

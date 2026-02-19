//! Webhook signature verification adapter for the integrations domain.
//!
//! Thin adapter layer that wires `platform/security` verifiers to source
//! system names. The verifier is selected at dispatch time based on the
//! `system` path parameter.
//!
//! ## Dispatch table
//!
//! | system     | verifier         | required env var           |
//! |------------|------------------|----------------------------|
//! | `stripe`   | `StripeVerifier` | `STRIPE_WEBHOOK_SECRET`    |
//! | `github`   | `GenericHmac`    | `GITHUB_WEBHOOK_SECRET`    |
//! | `internal` | `NoopVerifier`   | —                          |
//!
//! Unknown system names return `WebhookError::UnsupportedSystem`.

use std::collections::HashMap;
use security::{
    GenericHmacVerifier, NoopVerifier, StripeVerifier, VerifyError, WebhookVerifier,
};

use super::models::WebhookError;

/// Verify the signature for an inbound webhook from `system`.
///
/// Looks up the appropriate verifier from environment configuration and
/// delegates to it. Called **before** any database writes.
pub fn verify_signature(
    system: &str,
    headers: &HashMap<String, String>,
    raw_body: &[u8],
) -> Result<(), WebhookError> {
    let verifier = resolve_verifier(system)?;
    verifier
        .verify(headers, raw_body)
        .map_err(|e| WebhookError::SignatureVerification(format_verify_error(&e)))
}

/// Returns the appropriate verifier for the given system.
fn resolve_verifier(system: &str) -> Result<Box<dyn WebhookVerifier>, WebhookError> {
    match system {
        "stripe" => {
            let secret = std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();
            Ok(Box::new(StripeVerifier::new(&secret)))
        }
        "github" => {
            let secret = std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default();
            Ok(Box::new(GenericHmacVerifier::new(
                &secret,
                "x-hub-signature-256",
                Some("sha256="),
            )))
        }
        "internal" => Ok(Box::new(NoopVerifier)),
        other => Err(WebhookError::UnsupportedSystem {
            system: other.to_string(),
        }),
    }
}

fn format_verify_error(e: &VerifyError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_system_always_passes() {
        let headers = HashMap::new();
        let result = verify_signature("internal", &headers, b"{}");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unknown_system_rejected() {
        let headers = HashMap::new();
        let result = verify_signature("acme-payments", &headers, b"{}");
        assert!(matches!(result, Err(WebhookError::UnsupportedSystem { .. })));
    }
}

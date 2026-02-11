use super::error::TilledError;
use super::TilledConfig;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Verify a Tilled webhook signature
///
/// # Arguments
///
/// * `raw_body` - The raw request body as a string
/// * `signature` - The signature from the request header
/// * `webhook_secret` - The webhook secret from Tilled
/// * `tolerance` - The maximum allowed time difference in seconds (default: 300)
///
/// # Returns
///
/// * `Ok(())` if the signature is valid
/// * `Err(TilledError::WebhookVerificationFailed)` if the signature is invalid
pub fn verify_webhook_signature(
    raw_body: &str,
    signature: &str,
    webhook_secret: &str,
    tolerance: Option<i64>,
) -> Result<(), TilledError> {
    let tolerance = tolerance.unwrap_or(300);

    // Parse signature header
    let parts: Vec<&str> = signature.split(',').collect();

    let timestamp_part = parts.iter()
        .find(|p| p.starts_with("t="))
        .ok_or(TilledError::WebhookVerificationFailed)?;

    let signature_part = parts.iter()
        .find(|p| p.starts_with("v1="))
        .ok_or(TilledError::WebhookVerificationFailed)?;

    let timestamp = timestamp_part.strip_prefix("t=")
        .ok_or(TilledError::WebhookVerificationFailed)?;

    let received_signature = signature_part.strip_prefix("v1=")
        .ok_or(TilledError::WebhookVerificationFailed)?;

    // Check timestamp tolerance (prevent replay attacks)
    let webhook_time = timestamp.parse::<i64>()
        .map_err(|_| TilledError::WebhookVerificationFailed)?;

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TilledError::WebhookVerificationFailed)?
        .as_secs() as i64;

    if (current_time - webhook_time).abs() > tolerance {
        return Err(TilledError::WebhookVerificationFailed);
    }

    // Calculate expected signature
    let signed_payload = format!("{}.{}", timestamp, raw_body);

    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|_| TilledError::WebhookVerificationFailed)?;

    mac.update(signed_payload.as_bytes());

    let expected_signature = hex::encode(mac.finalize().into_bytes());

    // Verify signature (constant-time comparison)
    if expected_signature.len() != received_signature.len() {
        return Err(TilledError::WebhookVerificationFailed);
    }

    let received_bytes = hex::decode(received_signature)
        .map_err(|_| TilledError::WebhookVerificationFailed)?;
    let expected_bytes = hex::decode(&expected_signature)
        .map_err(|_| TilledError::WebhookVerificationFailed)?;

    if received_bytes.len() != expected_bytes.len() {
        return Err(TilledError::WebhookVerificationFailed);
    }

    // Constant-time comparison
    let mut result = 0u8;
    for (a, b) in received_bytes.iter().zip(expected_bytes.iter()) {
        result |= a ^ b;
    }

    if result == 0 {
        Ok(())
    } else {
        Err(TilledError::WebhookVerificationFailed)
    }
}

impl TilledConfig {
    /// Verify a webhook signature using this config's webhook secret
    pub fn verify_webhook(
        &self,
        raw_body: &str,
        signature: &str,
        tolerance: Option<i64>,
    ) -> Result<(), TilledError> {
        verify_webhook_signature(raw_body, signature, &self.webhook_secret, tolerance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_signature_verification() {
        let webhook_secret = "whsec_test_secret";
        let raw_body = r#"{"type":"payment_intent.succeeded","data":{"id":"pi_123"}}"#;

        // Generate a valid signature
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let signed_payload = format!("{}.{}", timestamp, raw_body);
        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let signature_hash = hex::encode(mac.finalize().into_bytes());

        let signature = format!("t={},v1={}", timestamp, signature_hash);

        // Verify the signature
        let result = verify_webhook_signature(raw_body, &signature, webhook_secret, Some(300));
        assert!(result.is_ok());
    }

    #[test]
    fn test_webhook_signature_verification_invalid() {
        let webhook_secret = "whsec_test_secret";
        let raw_body = r#"{"type":"payment_intent.succeeded","data":{"id":"pi_123"}}"#;
        let signature = "t=123456789,v1=invalid_signature";

        let result = verify_webhook_signature(raw_body, signature, webhook_secret, Some(300));
        assert!(result.is_err());
    }

    #[test]
    fn test_webhook_signature_verification_expired() {
        let webhook_secret = "whsec_test_secret";
        let raw_body = r#"{"type":"payment_intent.succeeded","data":{"id":"pi_123"}}"#;

        // Use an old timestamp (more than tolerance)
        let old_timestamp = 1000000;
        let signed_payload = format!("{}.{}", old_timestamp, raw_body);
        let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let signature_hash = hex::encode(mac.finalize().into_bytes());

        let signature = format!("t={},v1={}", old_timestamp, signature_hash);

        let result = verify_webhook_signature(raw_body, &signature, webhook_secret, Some(300));
        assert!(result.is_err());
    }
}

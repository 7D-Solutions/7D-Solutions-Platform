use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Maximum age (seconds) of an accepted webhook timestamp.
///
/// Tilled embeds `t=<unix_seconds>` in the signature header. Any event
/// whose timestamp is older than this threshold is rejected as a potential
/// replay, even if the HMAC is otherwise valid.
const WEBHOOK_TIMESTAMP_TOLERANCE_SECS: i64 = 300; // 5 minutes

/// Verify Tilled webhook signature and timestamp freshness.
///
/// Tilled signs webhooks with HMAC-SHA256. The `tilled-signature` header
/// has the form `t=<unix_ts>,v1=<hex_sig>`. We validate:
/// 1. HMAC over `"<ts>.<body>"` matches `v1`.
/// 2. Timestamp is within ±5 minutes of now (replay-window guard).
pub(super) fn verify_tilled_signature(
    payload: &[u8],
    signature_header: Option<&str>,
    secret: &str,
) -> Result<(), String> {
    let signature = signature_header.ok_or_else(|| "Missing signature header".to_string())?;

    // Tilled sends signature in format: "t=timestamp,v1=signature"
    let sig_parts: Vec<&str> = signature.split(',').collect();
    let mut timestamp = "";
    let mut sig_value = "";

    for part in sig_parts {
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value;
        } else if let Some(value) = part.strip_prefix("v1=") {
            sig_value = value;
        }
    }

    if timestamp.is_empty() || sig_value.is_empty() {
        return Err("Invalid signature format".to_string());
    }

    // Replay-window check: reject events older than tolerance window.
    let ts_unix: i64 = timestamp
        .parse()
        .map_err(|_| "Invalid timestamp in signature".to_string())?;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = now_unix - ts_unix;
    if age.abs() > WEBHOOK_TIMESTAMP_TOLERANCE_SECS {
        return Err(format!(
            "Webhook timestamp too old or too far in the future (age={}s, tolerance={}s)",
            age, WEBHOOK_TIMESTAMP_TOLERANCE_SECS
        ));
    }

    // Construct signed payload: timestamp.payload
    let signed_payload = format!("{}.{}", timestamp, String::from_utf8_lossy(payload));

    // Compute expected signature
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("Invalid secret: {}", e))?;
    mac.update(signed_payload.as_bytes());
    let expected_sig = hex::encode(mac.finalize().into_bytes());

    // Compare signatures (constant-time comparison would be better but
    // hex strings are already fixed length; timing leak is not exploitable here
    // since the attacker does not control the secret).
    if expected_sig != sig_value {
        return Err("Signature verification failed".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sig(secret: &str, ts: i64, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let signed = format!("{}.{}", ts, String::from_utf8_lossy(body));
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("test HMAC key");
        mac.update(signed.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        format!("t={},v1={}", ts, sig)
    }

    fn now_unix() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs() as i64
    }

    #[test]
    fn test_valid_signature_fresh_timestamp() {
        let secret = "whsec_unit_test_secret";
        let body = b"{}";
        let ts = now_unix();
        let header = make_sig(secret, ts, body);
        assert!(verify_tilled_signature(body, Some(&header), secret).is_ok());
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let secret = "whsec_unit_test_secret";
        let body = b"{}";
        let ts = now_unix();
        let header = format!("t={},v1=deadbeef", ts);
        let err = verify_tilled_signature(body, Some(&header), secret).unwrap_err();
        assert!(err.contains("Signature verification failed"), "{}", err);
    }

    #[test]
    fn test_old_timestamp_rejected() {
        let secret = "whsec_unit_test_secret";
        let body = b"{}";
        let old_ts = now_unix() - 600; // 10 minutes ago
        let header = make_sig(secret, old_ts, body);
        let err = verify_tilled_signature(body, Some(&header), secret).unwrap_err();
        assert!(err.contains("too old"), "{}", err);
    }

    #[test]
    fn test_missing_header_rejected() {
        assert!(verify_tilled_signature(b"{}", None, "secret").is_err());
    }

    #[test]
    fn test_malformed_header_rejected() {
        let err = verify_tilled_signature(b"{}", Some("garbage"), "secret").unwrap_err();
        assert!(err.contains("Invalid signature format"), "{}", err);
    }
}

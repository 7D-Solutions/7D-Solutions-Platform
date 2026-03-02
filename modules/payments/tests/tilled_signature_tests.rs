/// Integration tests for Tilled webhook signature verification.
///
/// Uses known-good test vectors (pre-computed HMAC-SHA256 values) to validate
/// the signature verification logic without calling the Tilled API.
use payments_rs::webhook_signature::{validate_webhook_signature, SignatureError, WebhookSource};
use std::collections::HashMap;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_sig(secret: &str, ts: i64, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
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

// ── tests ─────────────────────────────────────────────────────────────────────

/// A correctly signed, fresh webhook must be accepted.
#[test]
fn test_valid_tilled_signature_accepted() {
    let secret = "whsec_test_abc123";
    let body = b"{\"type\":\"payment_intent.succeeded\",\"id\":\"pi_test_001\"}";
    let ts = now_ts();
    let sig_header = make_sig(secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[secret]);
    assert!(
        result.is_ok(),
        "Valid signature must be accepted: {:?}",
        result
    );
}

/// Tampered body must be rejected even with a valid-looking header.
#[test]
fn test_tampered_body_rejected() {
    let secret = "whsec_test_abc123";
    let original_body = b"{\"amount\":1000}";
    let tampered_body = b"{\"amount\":9999}";
    let ts = now_ts();
    let sig_header = make_sig(secret, ts, original_body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result =
        validate_webhook_signature(WebhookSource::Tilled, &headers, tampered_body, &[secret]);
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Tampered body must be rejected"
    );
}

/// A wrong secret must cause rejection.
#[test]
fn test_wrong_secret_rejected() {
    let signing_secret = "whsec_real_secret";
    let wrong_secret = "whsec_wrong_secret";
    let body = b"{\"type\":\"payment_intent.failed\"}";
    let ts = now_ts();
    let sig_header = make_sig(signing_secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[wrong_secret]);
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Wrong secret must be rejected"
    );
}

/// A webhook older than 5 minutes must be rejected as a replay.
#[test]
fn test_replay_rejected_old_timestamp() {
    let secret = "whsec_test_abc123";
    let body = b"{\"type\":\"payment_intent.succeeded\"}";
    let stale_ts = now_ts() - 310; // 310 seconds in the past (> 5 min)
    let sig_header = make_sig(secret, stale_ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[secret]);
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { ref reason }) if reason.contains("replay")),
        "Stale timestamp must be rejected as replay"
    );
}

/// A future timestamp beyond tolerance must also be rejected.
#[test]
fn test_replay_rejected_future_timestamp() {
    let secret = "whsec_test_abc123";
    let body = b"{}";
    let future_ts = now_ts() + 310;
    let sig_header = make_sig(secret, future_ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, body, &[secret]);
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Future timestamp beyond tolerance must be rejected"
    );
}

/// Missing header must produce MissingSignature.
#[test]
fn test_missing_header_produces_error() {
    let headers = HashMap::new();
    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, b"{}", &["secret"]);
    assert_eq!(
        result.unwrap_err(),
        SignatureError::MissingSignature,
        "Missing header must return MissingSignature"
    );
}

/// Malformed header (no t= or v1=) must be rejected.
#[test]
fn test_malformed_header_rejected() {
    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), "notavalidsig".to_string());

    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, b"{}", &["secret"]);
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Malformed header must be rejected"
    );
}

/// No configured secret must return InvalidSignature with clear message.
#[test]
fn test_no_secret_configured() {
    let mut headers = HashMap::new();
    headers.insert(
        "tilled-signature".to_string(),
        "t=123456,v1=abc".to_string(),
    );

    let result = validate_webhook_signature(WebhookSource::Tilled, &headers, b"{}", &[]);
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { ref reason }) if reason.contains("not configured")),
        "Missing secret must produce clear error"
    );
}

/// Provider selection test: PAYMENTS_PROVIDER=mock should keep mock behaviour.
#[test]
fn test_provider_selection_mock() {
    use payments_rs::PaymentsProvider;
    assert_eq!(
        PaymentsProvider::from_str("mock").unwrap(),
        PaymentsProvider::Mock
    );
    assert_eq!(
        PaymentsProvider::from_str("MOCK").unwrap(),
        PaymentsProvider::Mock
    );
}

/// Provider selection test: PAYMENTS_PROVIDER=tilled selects Tilled.
#[test]
fn test_provider_selection_tilled() {
    use payments_rs::PaymentsProvider;
    assert_eq!(
        PaymentsProvider::from_str("tilled").unwrap(),
        PaymentsProvider::Tilled
    );
}

/// Provider selection rejects unknown values.
#[test]
fn test_provider_selection_invalid() {
    use payments_rs::PaymentsProvider;
    assert!(PaymentsProvider::from_str("stripe").is_err());
    assert!(PaymentsProvider::from_str("").is_err());
}

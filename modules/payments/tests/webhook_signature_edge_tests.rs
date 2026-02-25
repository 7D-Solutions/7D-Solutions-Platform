/// Edge-case tests for webhook signature verification (bd-3fvu).
///
/// Covers: HMAC correctness edge cases, replay attack boundary conditions,
/// malformed payload handling, and secret rotation scenarios.
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

// ── HMAC verification correctness ────────────────────────────────────────────

/// Case-insensitive header lookup: "Tilled-Signature" must be recognised.
#[test]
fn test_case_insensitive_header_tilled_signature() {
    let secret = "whsec_case_test";
    let body = b"{\"id\":\"pi_case\"}";
    let ts = now_ts();
    let sig_header = make_sig(secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("Tilled-Signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[secret],
    );
    assert!(result.is_ok(), "Tilled-Signature (mixed case) must be accepted: {:?}", result);
}

/// Empty body must still verify correctly when signature matches.
#[test]
fn test_empty_body_valid_signature() {
    let secret = "whsec_empty_body";
    let body = b"";
    let ts = now_ts();
    let sig_header = make_sig(secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[secret],
    );
    assert!(result.is_ok(), "Empty body with valid signature must pass: {:?}", result);
}

/// Secret rotation: webhook signed with OLD secret accepted when both secrets provided.
#[test]
fn test_secret_rotation_old_secret_accepted() {
    let old_secret = "whsec_old_rotation";
    let new_secret = "whsec_new_rotation";
    let body = b"{\"type\":\"charge.succeeded\"}";
    let ts = now_ts();
    let sig_header = make_sig(old_secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[new_secret, old_secret],
    );
    assert!(result.is_ok(), "Old secret must be accepted during rotation: {:?}", result);
}

/// Secret rotation: webhook signed with NEW secret accepted when both secrets provided.
#[test]
fn test_secret_rotation_new_secret_accepted() {
    let old_secret = "whsec_old_rotation";
    let new_secret = "whsec_new_rotation";
    let body = b"{\"type\":\"charge.succeeded\"}";
    let ts = now_ts();
    let sig_header = make_sig(new_secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[new_secret, old_secret],
    );
    assert!(result.is_ok(), "New secret must be accepted during rotation: {:?}", result);
}

/// Secret rotation: unknown secret rejected when neither configured secret matches.
#[test]
fn test_secret_rotation_unknown_rejected() {
    let body = b"{\"type\":\"charge.failed\"}";
    let ts = now_ts();
    let sig_header = make_sig("whsec_attacker", ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &["whsec_new", "whsec_old"],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Unknown secret must be rejected during rotation"
    );
}

/// Internal webhooks always pass regardless of headers or body.
#[test]
fn test_internal_always_passes() {
    let result = validate_webhook_signature(
        WebhookSource::Internal,
        &HashMap::new(),
        b"anything",
        &[],
    );
    assert!(result.is_ok(), "Internal webhook must always pass");
}

/// Stripe source must return UnsupportedSource.
#[test]
fn test_stripe_unsupported() {
    let result = validate_webhook_signature(
        WebhookSource::Stripe,
        &HashMap::new(),
        b"{}",
        &[],
    );
    assert_eq!(
        result.unwrap_err(),
        SignatureError::UnsupportedSource { source: "stripe".to_string() }
    );
}

// ── Replay attack prevention ─────────────────────────────────────────────────

/// Timestamp exactly at the 300s boundary should still be accepted.
#[test]
fn test_timestamp_at_boundary_accepted() {
    let secret = "whsec_boundary";
    let body = b"{\"id\":\"pi_edge\"}";
    let ts = now_ts() - 299; // 299s ago, within 300s tolerance
    let sig_header = make_sig(secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[secret],
    );
    assert!(result.is_ok(), "Timestamp 299s old must be accepted: {:?}", result);
}

/// Timestamp just outside boundary (301s) must be rejected.
#[test]
fn test_timestamp_just_outside_boundary_rejected() {
    let secret = "whsec_boundary";
    let body = b"{\"id\":\"pi_edge\"}";
    let ts = now_ts() - 301; // 301s ago, outside 300s tolerance
    let sig_header = make_sig(secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[secret],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { ref reason }) if reason.contains("replay")),
        "Timestamp 301s old must be rejected as replay"
    );
}

/// Future timestamp just outside boundary must be rejected.
#[test]
fn test_future_timestamp_just_outside_boundary_rejected() {
    let secret = "whsec_boundary";
    let body = b"{}";
    let ts = now_ts() + 301;
    let sig_header = make_sig(secret, ts, body);

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[secret],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Future timestamp 301s ahead must be rejected"
    );
}

/// Non-numeric timestamp must be rejected.
#[test]
fn test_non_numeric_timestamp_rejected() {
    let mut headers = HashMap::new();
    headers.insert(
        "tilled-signature".to_string(),
        "t=notanumber,v1=deadbeef".to_string(),
    );

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        b"{}",
        &["secret"],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { ref reason }) if reason.contains("timestamp")),
        "Non-numeric timestamp must be rejected"
    );
}

// ── Malformed payload handling ───────────────────────────────────────────────

/// Header with only t= but no v1= must be rejected.
#[test]
fn test_header_missing_v1_rejected() {
    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), "t=1234567890".to_string());

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        b"{}",
        &["secret"],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Header missing v1= must be rejected"
    );
}

/// Header with only v1= but no t= must be rejected.
#[test]
fn test_header_missing_timestamp_rejected() {
    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), "v1=deadbeef".to_string());

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        b"{}",
        &["secret"],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Header missing t= must be rejected"
    );
}

/// Empty header value must be rejected.
#[test]
fn test_empty_header_value_rejected() {
    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), "".to_string());

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        b"{}",
        &["secret"],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Empty header must be rejected"
    );
}

/// Header with extra unknown components must still parse t= and v1= correctly.
#[test]
fn test_header_with_extra_components_accepted() {
    let secret = "whsec_extra";
    let body = b"{\"id\":\"pi_extra\"}";
    let ts = now_ts();
    let sig_header = {
        let raw = make_sig(secret, ts, body);
        format!("{},extra=something", raw)
    };

    let mut headers = HashMap::new();
    headers.insert("tilled-signature".to_string(), sig_header);

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        body,
        &[secret],
    );
    assert!(result.is_ok(), "Extra header components must not break parsing: {:?}", result);
}

/// Header with t= and v1= but v1 is empty must be rejected.
#[test]
fn test_empty_v1_value_rejected() {
    let mut headers = HashMap::new();
    headers.insert(
        "tilled-signature".to_string(),
        format!("t={},v1=", now_ts()),
    );

    let result = validate_webhook_signature(
        WebhookSource::Tilled,
        &headers,
        b"{}",
        &["secret"],
    );
    assert!(
        matches!(result, Err(SignatureError::InvalidSignature { .. })),
        "Empty v1 value must be rejected"
    );
}

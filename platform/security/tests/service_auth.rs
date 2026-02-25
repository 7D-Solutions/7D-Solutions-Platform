//! Integration tests for service-to-service HMAC authentication.
//!
//! Tests generate and verify real HMAC-SHA256 tokens — no mocks.
//!
//! Because `sign_claims()` reads SERVICE_AUTH_SECRET from the process env,
//! and Rust tests run in parallel within a single process, we serialise
//! all env-mutating tests behind a global mutex.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use std::sync::Mutex;

use security::service_auth::{
    generate_service_token, verify_service_token, ServiceAuthClaims, ServiceAuthError,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Run `f` with SERVICE_AUTH_SECRET set to `secret`, then restore.
fn with_secret<F, R>(secret: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("SERVICE_AUTH_SECRET", secret);
    let result = f();
    std::env::remove_var("SERVICE_AUTH_SECRET");
    result
}

/// Helper to manually sign claims for expiration / future-dated tests.
fn manual_sign(claims_b64: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(claims_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(&sig)
}

// ── Generate + verify round-trip ─────────────────────────────────────────

#[test]
fn generate_and_verify_roundtrip() {
    with_secret("test-secret-roundtrip", || {
        let token = generate_service_token("billing-service", None).unwrap();
        let claims = verify_service_token(&token).unwrap();

        assert_eq!(claims.service_name, "billing-service");
        assert!(claims.expires_at > claims.issued_at);
    });
}

#[test]
fn default_validity_is_15_minutes() {
    with_secret("test-secret-validity", || {
        let token = generate_service_token("gl-service", None).unwrap();
        let claims = verify_service_token(&token).unwrap();

        let validity = claims.expires_at - claims.issued_at;
        assert_eq!(validity, 15 * 60);
    });
}

#[test]
fn custom_validity_applied() {
    with_secret("test-secret-custom", || {
        let token = generate_service_token("ar-service", Some(30)).unwrap();
        let claims = verify_service_token(&token).unwrap();

        let validity = claims.expires_at - claims.issued_at;
        assert_eq!(validity, 30 * 60);
    });
}

// ── Token format ─────────────────────────────────────────────────────────

#[test]
fn token_has_two_dot_separated_parts() {
    with_secret("test-secret-format", || {
        let token = generate_service_token("test", None).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 2, "Token should be <claims>.<signature>");
    });
}

#[test]
fn claims_part_is_valid_base64_json() {
    with_secret("test-secret-b64", || {
        let token = generate_service_token("test-svc", None).unwrap();
        let claims_b64 = token.split('.').next().unwrap();
        let json_bytes = URL_SAFE_NO_PAD.decode(claims_b64).unwrap();
        let claims: ServiceAuthClaims = serde_json::from_slice(&json_bytes).unwrap();
        assert_eq!(claims.service_name, "test-svc");
    });
}

// ── Invalid tokens ───────────────────────────────────────────────────────

#[test]
fn invalid_format_no_dot() {
    with_secret("test-secret-invalid", || {
        let result = verify_service_token("nodottoken");
        assert!(matches!(result, Err(ServiceAuthError::InvalidFormat)));
    });
}

#[test]
fn invalid_format_too_many_dots() {
    with_secret("test-secret-dots", || {
        let result = verify_service_token("a.b.c");
        assert!(matches!(result, Err(ServiceAuthError::InvalidFormat)));
    });
}

#[test]
fn tampered_claims_rejected() {
    with_secret("test-secret-tamper", || {
        let token = generate_service_token("legit", None).unwrap();
        let parts: Vec<&str> = token.split('.').collect();

        let tampered_claims = ServiceAuthClaims {
            service_name: "evil".to_string(),
            issued_at: Utc::now().timestamp(),
            expires_at: Utc::now().timestamp() + 9999,
        };
        let tampered_b64 =
            URL_SAFE_NO_PAD.encode(serde_json::to_string(&tampered_claims).unwrap());
        let tampered_token = format!("{}.{}", tampered_b64, parts[1]);

        let result = verify_service_token(&tampered_token);
        assert!(matches!(result, Err(ServiceAuthError::InvalidSignature)));
    });
}

#[test]
fn wrong_secret_rejects_token() {
    let token = with_secret("secret-A", || {
        generate_service_token("svc", None).unwrap()
    });

    with_secret("secret-B", || {
        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::InvalidSignature)));
    });
}

// ── Expiration ───────────────────────────────────────────────────────────

#[test]
fn expired_token_rejected() {
    let secret = "test-secret-expired";
    with_secret(secret, || {
        let now = Utc::now().timestamp();
        let claims = ServiceAuthClaims {
            service_name: "old-svc".to_string(),
            issued_at: now - 7200,
            expires_at: now - 3600,
        };

        let claims_json = serde_json::to_string(&claims).unwrap();
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        let sig_b64 = manual_sign(&claims_b64, secret);
        let token = format!("{}.{}", claims_b64, sig_b64);

        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::TokenExpired)));
    });
}

#[test]
fn future_issued_token_rejected() {
    let secret = "test-secret-future";
    with_secret(secret, || {
        let now = Utc::now().timestamp();
        let claims = ServiceAuthClaims {
            service_name: "future-svc".to_string(),
            issued_at: now + 300,
            expires_at: now + 1200,
        };

        let claims_json = serde_json::to_string(&claims).unwrap();
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        let sig_b64 = manual_sign(&claims_b64, secret);
        let token = format!("{}.{}", claims_b64, sig_b64);

        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::TokenNotYetValid)));
    });
}

// ── Missing secret ───────────────────────────────────────────────────────

#[test]
fn missing_secret_returns_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("SERVICE_AUTH_SECRET");
    let result = generate_service_token("svc", None);
    assert!(matches!(result, Err(ServiceAuthError::MissingSigningKey)));
}

// ── Service name preserved ───────────────────────────────────────────────

#[test]
fn various_service_names_preserved() {
    with_secret("test-secret-names", || {
        for name in ["tenantctl", "ar-service", "gl-service", "inventory-rs"] {
            let token = generate_service_token(name, None).unwrap();
            let claims = verify_service_token(&token).unwrap();
            assert_eq!(claims.service_name, name);
        }
    });
}

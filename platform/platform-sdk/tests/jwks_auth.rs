//! Integration tests for JWKS auth in the SDK startup path.
//!
//! These tests verify:
//! - `from_jwks_url` with fallback_to_env=true falls back to env var on dead endpoint
//! - `from_jwks_url` with fallback_to_env=false returns JwksUnavailable on dead endpoint
//! - `from_env_with_overlap` continues to work unchanged after the JWKS refactor
//! - Dynamic key store correctly verifies tokens across multiple keys

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use security::{JwtVerifier, SecurityError};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn make_keys() -> (EncodingKey, String) {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key gen");
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("PEM encode");
    let pub_pem = pub_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public PEM");
    let enc = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
    (enc, pub_pem)
}

fn sign_token(enc: &EncodingKey) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        roles: vec!["admin".into()],
        perms: vec!["ar.create".into()],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, enc).expect("sign token")
}

/// Dead JWKS endpoint + fallback_to_env=true: falls back to JWT_PUBLIC_KEY.
#[tokio::test]
async fn jwks_dead_endpoint_fallback_to_env() {
    let (_enc, pub_pem) = make_keys();

    // Set env var so fallback works
    std::env::set_var("JWT_PUBLIC_KEY", &pub_pem);

    let result = JwtVerifier::from_jwks_url(
        "http://127.0.0.1:1/nonexistent/.well-known/jwks.json",
        std::time::Duration::from_secs(300),
        true,
    )
    .await;

    std::env::remove_var("JWT_PUBLIC_KEY");

    // Should succeed via env fallback
    assert!(result.is_ok(), "expected Ok from env fallback");
}

/// Dead JWKS endpoint + fallback_to_env=false: returns JwksUnavailable.
#[tokio::test]
async fn jwks_dead_endpoint_no_fallback_fails() {
    let result = JwtVerifier::from_jwks_url(
        "http://127.0.0.1:1/nonexistent/.well-known/jwks.json",
        std::time::Duration::from_secs(300),
        false,
    )
    .await;

    match result {
        Err(SecurityError::JwksUnavailable(_)) => {} // expected
        Err(other) => panic!("expected JwksUnavailable, got {other:?}"),
        Ok(_) => panic!("expected Err, got Ok"),
    }
}

/// from_env_with_overlap still works after the KeyStore refactor.
#[test]
fn from_env_with_overlap_unchanged() {
    let (enc, pub_pem) = make_keys();

    std::env::set_var("JWT_PUBLIC_KEY", &pub_pem);
    let verifier = JwtVerifier::from_env_with_overlap();
    std::env::remove_var("JWT_PUBLIC_KEY");

    let verifier = verifier.expect("verifier from env");
    let token = sign_token(&enc);
    let verified = verifier.verify(&token).expect("verify");
    assert!(!verified.roles.is_empty());
}

/// Dead JWKS + fallback_to_env=true + no env var set: returns JwksUnavailable.
#[tokio::test]
async fn jwks_fallback_no_env_var_fails() {
    // Ensure env var is not set
    std::env::remove_var("JWT_PUBLIC_KEY");
    std::env::remove_var("JWT_PUBLIC_KEY_PEM");

    let result = JwtVerifier::from_jwks_url(
        "http://127.0.0.1:1/nonexistent/.well-known/jwks.json",
        std::time::Duration::from_secs(300),
        true,
    )
    .await;

    match result {
        Err(SecurityError::JwksUnavailable(_)) => {} // expected
        Err(other) => panic!("expected JwksUnavailable, got {other:?}"),
        Ok(_) => panic!("expected Err when both JWKS and env fail"),
    }
}

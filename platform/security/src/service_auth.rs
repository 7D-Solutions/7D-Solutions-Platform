//! Service-to-service authentication using signed tokens
//!
//! This module provides HMAC-SHA256 signed tokens for internal service authentication.
//! Tokens are short-lived (15 minutes) and include the service name and timestamp.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::env;

type HmacSha256 = Hmac<Sha256>;

/// Service authentication claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAuthClaims {
    /// Name of the calling service (e.g., "tenantctl", "ar-service", "gl-service")
    pub service_name: String,

    /// Token issuance timestamp (Unix seconds)
    pub issued_at: i64,

    /// Token expiration timestamp (Unix seconds)
    pub expires_at: i64,
}

/// Service authentication errors
#[derive(Debug, thiserror::Error)]
pub enum ServiceAuthError {
    #[error("Invalid token format")]
    InvalidFormat,

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Token expired")]
    TokenExpired,

    #[error("Token not yet valid")]
    TokenNotYetValid,

    #[error("Missing signing key")]
    MissingSigningKey,

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),
}

/// Generate a signed service token
///
/// # Arguments
/// * `service_name` - Name of the service requesting the token
/// * `validity_minutes` - Token validity in minutes (default: 15)
///
/// # Returns
/// A base64-encoded signed token in the format: `<claims>.<signature>`
///
/// # Errors
/// Returns an error if the signing key is not set
pub fn generate_service_token(
    service_name: &str,
    validity_minutes: Option<i64>,
) -> Result<String, ServiceAuthError> {
    let validity = validity_minutes.unwrap_or(15);
    let now = Utc::now();
    let expires_at = now + Duration::minutes(validity);

    let claims = ServiceAuthClaims {
        service_name: service_name.to_string(),
        issued_at: now.timestamp(),
        expires_at: expires_at.timestamp(),
    };

    // Serialize claims
    let claims_json = serde_json::to_string(&claims)?;
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());

    // Sign claims
    let signature = sign_claims(&claims_b64)?;
    let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);

    // Return token: claims.signature
    Ok(format!("{}.{}", claims_b64, signature_b64))
}

/// Verify a signed service token
///
/// # Arguments
/// * `token` - The signed token to verify
///
/// # Returns
/// The verified claims if the token is valid
///
/// # Errors
/// Returns an error if the token is invalid, expired, or has an invalid signature
pub fn verify_service_token(token: &str) -> Result<ServiceAuthClaims, ServiceAuthError> {
    // Split token into claims and signature
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return Err(ServiceAuthError::InvalidFormat);
    }

    let claims_b64 = parts[0];
    let signature_b64 = parts[1];

    // Verify signature (constant-time comparison to prevent timing attacks)
    let expected_signature = sign_claims(claims_b64)?;
    let actual_signature = URL_SAFE_NO_PAD.decode(signature_b64)?;

    if expected_signature.len() != actual_signature.len()
        || !constant_time_eq(&expected_signature, &actual_signature)
    {
        return Err(ServiceAuthError::InvalidSignature);
    }

    // Decode claims
    let claims_json = URL_SAFE_NO_PAD.decode(claims_b64)?;
    let claims: ServiceAuthClaims = serde_json::from_slice(&claims_json)?;

    // Verify expiration
    let now = Utc::now().timestamp();
    if claims.expires_at < now {
        return Err(ServiceAuthError::TokenExpired);
    }

    if claims.issued_at > now + 60 {
        // Allow 60 seconds clock skew
        return Err(ServiceAuthError::TokenNotYetValid);
    }

    Ok(claims)
}

/// Constant-time byte slice comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Sign claims using HMAC-SHA256
fn sign_claims(claims_b64: &str) -> Result<Vec<u8>, ServiceAuthError> {
    let secret =
        env::var("SERVICE_AUTH_SECRET").map_err(|_| ServiceAuthError::MissingSigningKey)?;

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take keys of any size");
    mac.update(claims_b64.as_bytes());

    Ok(mac.finalize().into_bytes().to_vec())
}

/// Mint a proper RSA-signed JWT that ClaimsLayer can verify, embedding caller context.
///
/// Uses `JWT_PRIVATE_KEY_PEM` to sign. The resulting token contains
/// `RawAccessClaims`-compatible fields so `JwtVerifier::verify()` can
/// decode it into `VerifiedClaims` with `service.internal` permission.
///
/// `tenant_id` and `actor_id` come from the incoming request's `VerifiedClaims`
/// so the receiving service sees real context, not nil UUIDs.
fn mint_rsa_service_jwt(
    service_name: &str,
    tenant_id: uuid::Uuid,
    actor_id: uuid::Uuid,
) -> Result<String, ServiceAuthError> {
    let pem = env::var("JWT_PRIVATE_KEY_PEM")
        .map_err(|_| ServiceAuthError::MissingSigningKey)?;
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|_| ServiceAuthError::MissingSigningKey)?;

    let now = Utc::now();
    let exp = now + Duration::minutes(15);

    let claims = serde_json::json!({
        "sub": actor_id.to_string(),
        "iss": "auth-rs",
        "aud": "7d-platform",
        "iat": now.timestamp(),
        "exp": exp.timestamp(),
        "jti": uuid::Uuid::new_v4().to_string(),
        "tenant_id": tenant_id.to_string(),
        "roles": [],
        "perms": ["service.internal"],
        "actor_type": "service",
        "actor_name": service_name,
        "ver": "1.0",
    });

    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &encoding_key)
        .map_err(|_| ServiceAuthError::MissingSigningKey)
}

/// Mint an RSA-signed JWT with caller context for cross-service calls through ClaimsLayer.
///
/// Call this when a service needs to call another service and must forward the
/// original request's `tenant_id` and `actor_id` so the receiving service's
/// `ClaimsLayer` sees real claims (not nil UUIDs).
///
/// Requires `JWT_PRIVATE_KEY_PEM` and `SERVICE_NAME` to be set.
pub fn mint_service_jwt_with_context(
    tenant_id: uuid::Uuid,
    actor_id: uuid::Uuid,
) -> Result<String, ServiceAuthError> {
    let service_name = env::var("SERVICE_NAME").unwrap_or_else(|_| "service".to_string());
    mint_rsa_service_jwt(&service_name, tenant_id, actor_id)
}

/// Get service token from environment or generate one
///
/// Prefers RSA-signed JWT (via `JWT_PRIVATE_KEY_PEM`) because that is what
/// `ClaimsLayer` / `JwtVerifier` can decode. Falls back to HMAC token
/// only if the private key is not available.
pub fn get_service_token() -> Result<String, ServiceAuthError> {
    // Check if token is already in environment
    if let Ok(token) = env::var("SERVICE_TOKEN") {
        if verify_service_token(&token).is_ok() {
            return Ok(token);
        }
    }

    let service_name = env::var("SERVICE_NAME").unwrap_or_else(|_| "unknown".to_string());

    // Prefer RSA JWT — compatible with ClaimsLayer verification.
    // No request context available here; use nil UUIDs as the service principal.
    // For context-aware tokens use mint_service_jwt_with_context().
    if env::var("JWT_PRIVATE_KEY_PEM").is_ok() {
        return mint_rsa_service_jwt(&service_name, uuid::Uuid::nil(), uuid::Uuid::nil());
    }

    // Fallback to HMAC (legacy, not compatible with ClaimsLayer)
    generate_service_token(&service_name, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup_test_env() {
        INIT.call_once(|| {
            env::set_var("SERVICE_AUTH_SECRET", "test-secret-key-for-hmac-signing");
        });
    }

    #[test]
    fn test_generate_and_verify_token() {
        setup_test_env();

        let token = generate_service_token("tenantctl", None).expect("generate");
        let claims = verify_service_token(&token).expect("verify");

        assert_eq!(claims.service_name, "tenantctl");
        assert!(claims.expires_at > claims.issued_at);
    }

    #[test]
    fn test_invalid_token_format() {
        setup_test_env();

        let result = verify_service_token("invalid-token");
        assert!(matches!(result, Err(ServiceAuthError::InvalidFormat)));
    }

    #[test]
    fn test_invalid_signature() {
        setup_test_env();

        let token = generate_service_token("tenantctl", None).expect("generate");
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[1] = "invalid-signature";
        let tampered_token = parts.join(".");

        let result = verify_service_token(&tampered_token);
        assert!(matches!(
            result,
            Err(ServiceAuthError::Base64Error(_)) | Err(ServiceAuthError::InvalidSignature)
        ));
    }

    #[test]
    fn test_expired_token() {
        setup_test_env();

        // Generate token with negative validity (already expired)
        let now = Utc::now();
        let claims = ServiceAuthClaims {
            service_name: "test".to_string(),
            issued_at: now.timestamp() - 3600,
            expires_at: now.timestamp() - 1800, // Expired 30 minutes ago
        };

        let claims_json = serde_json::to_string(&claims).expect("serialize");
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        let signature = sign_claims(&claims_b64).expect("sign");
        let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);
        let token = format!("{}.{}", claims_b64, signature_b64);

        let result = verify_service_token(&token);
        assert!(matches!(result, Err(ServiceAuthError::TokenExpired)));
    }

    #[test]
    fn test_custom_validity() {
        setup_test_env();

        let token = generate_service_token("test-service", Some(30)).expect("generate");
        let claims = verify_service_token(&token).expect("verify");

        let validity_seconds = claims.expires_at - claims.issued_at;
        assert_eq!(validity_seconds, 30 * 60); // 30 minutes in seconds
    }
}

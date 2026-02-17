//! E2E tests for service-to-service authentication
//!
//! These tests verify that:
//! 1. Services can generate valid authentication tokens
//! 2. Token verification correctly validates and rejects tokens
//! 3. Expired tokens are rejected
//! 4. Tampered tokens are rejected

use security::{generate_service_token, verify_service_token, ServiceAuthError};
use std::env;
use std::sync::Once;

static INIT: Once = Once::new();

fn setup_test_env() {
    INIT.call_once(|| {
        env::set_var("SERVICE_AUTH_SECRET", "e2e-test-secret-key-for-service-auth");
        env::set_var("SERVICE_NAME", "tenantctl");
    });
}

#[tokio::test]
async fn test_generate_valid_token() {
    setup_test_env();

    let token = generate_service_token("tenantctl", None)
        .expect("Failed to generate service token");

    assert!(!token.is_empty());
    assert!(token.contains('.'), "Token should contain a signature separator");

    println!("✅ Generated valid service token");
}

#[tokio::test]
async fn test_verify_valid_token() {
    setup_test_env();

    let token = generate_service_token("tenantctl", None)
        .expect("Failed to generate service token");

    let claims = verify_service_token(&token)
        .expect("Failed to verify service token");

    assert_eq!(claims.service_name, "tenantctl");
    assert!(claims.expires_at > claims.issued_at);

    println!("✅ Verified valid service token");
    println!("   Service: {}", claims.service_name);
    println!("   Issued at: {}", claims.issued_at);
    println!("   Expires at: {}", claims.expires_at);
}

#[tokio::test]
async fn test_reject_invalid_format() {
    setup_test_env();

    let result = verify_service_token("invalid-token-without-signature");

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ServiceAuthError::InvalidFormat));

    println!("✅ Rejected token with invalid format");
}

#[tokio::test]
async fn test_reject_tampered_token() {
    setup_test_env();

    let token = generate_service_token("tenantctl", None)
        .expect("Failed to generate service token");

    // Tamper with the token by modifying the claims
    let parts: Vec<&str> = token.split('.').collect();
    let tampered_token = format!("eyJzZXJ2aWNlX25hbWUiOiJoYWNrZXIifQ.{}", parts[1]);

    let result = verify_service_token(&tampered_token);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ServiceAuthError::InvalidSignature),
        "Expected InvalidSignature, got: {:?}", err
    );

    println!("✅ Rejected tampered token");
}

#[tokio::test]
async fn test_reject_expired_token() {
    setup_test_env();

    use chrono::Utc;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use security::ServiceAuthClaims;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    // Create an expired token
    let now = Utc::now();
    let expired_claims = ServiceAuthClaims {
        service_name: "test".to_string(),
        issued_at: now.timestamp() - 3600,      // 1 hour ago
        expires_at: now.timestamp() - 1800,     // 30 minutes ago (expired)
    };

    let claims_json = serde_json::to_string(&expired_claims).unwrap();
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());

    // Sign it properly
    let secret = env::var("SERVICE_AUTH_SECRET").unwrap();
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(claims_b64.as_bytes());
    let signature = mac.finalize().into_bytes();
    let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);

    let expired_token = format!("{}.{}", claims_b64, signature_b64);

    let result = verify_service_token(&expired_token);

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ServiceAuthError::TokenExpired));

    println!("✅ Rejected expired token");
}

#[tokio::test]
async fn test_token_with_custom_validity() {
    setup_test_env();

    // Generate token with 30-minute validity
    let token = generate_service_token("ar-service", Some(30))
        .expect("Failed to generate service token");

    let claims = verify_service_token(&token)
        .expect("Failed to verify service token");

    let validity_seconds = claims.expires_at - claims.issued_at;
    assert_eq!(validity_seconds, 30 * 60, "Token should have 30-minute validity");

    println!("✅ Verified token with custom validity");
    println!("   Validity: {} minutes", validity_seconds / 60);
}

#[tokio::test]
async fn test_multiple_services_can_authenticate() {
    setup_test_env();

    let services = vec!["tenantctl", "ar-service", "gl-service", "payments-service"];

    for service_name in services {
        let token = generate_service_token(service_name, None)
            .expect(&format!("Failed to generate token for {}", service_name));

        let claims = verify_service_token(&token)
            .expect(&format!("Failed to verify token for {}", service_name));

        assert_eq!(claims.service_name, service_name);

        println!("✅ Service '{}' authenticated successfully", service_name);
    }
}

#[tokio::test]
async fn test_get_service_token_from_env() {
    setup_test_env();

    // Test when SERVICE_TOKEN is not set (should generate new token)
    env::remove_var("SERVICE_TOKEN");

    let token1 = security::get_service_token()
        .expect("Failed to get service token");

    let claims1 = verify_service_token(&token1)
        .expect("Failed to verify first token");

    assert_eq!(claims1.service_name, "tenantctl"); // From SERVICE_NAME env var

    // Test when SERVICE_TOKEN is set (should use existing token)
    env::set_var("SERVICE_TOKEN", &token1);

    let token2 = security::get_service_token()
        .expect("Failed to get service token from env");

    assert_eq!(token1, token2, "Should reuse existing valid token from environment");

    println!("✅ get_service_token() works correctly");

    // Cleanup
    env::remove_var("SERVICE_TOKEN");
}

/// Integration test: Simulate HTTP request with auth header
#[tokio::test]
async fn test_http_request_simulation() {
    setup_test_env();

    // Generate token
    let token = generate_service_token("tenantctl", None)
        .expect("Failed to generate service token");

    // Simulate server-side: extract Bearer token from header
    let auth_header = format!("Bearer {}", token);
    let bearer_prefix = "Bearer ";

    assert!(auth_header.starts_with(bearer_prefix));

    let extracted_token = auth_header.strip_prefix(bearer_prefix).unwrap();

    // Verify the extracted token
    let claims = verify_service_token(extracted_token)
        .expect("Failed to verify extracted token");

    assert_eq!(claims.service_name, "tenantctl");

    println!("✅ HTTP Authorization header simulation successful");
    println!("   Header: Authorization: {}", auth_header);
    println!("   Verified service: {}", claims.service_name);
}

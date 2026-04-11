//! E2E tests for service-to-service JWT auth with tenant/actor context.
//!
//! Verifies that:
//! 1. `mint_service_jwt_with_context` embeds tenant_id and actor_id in the JWT
//! 2. A receiving service's ClaimsLayer (strict mode) accepts the minted JWT
//! 3. A missing token is rejected with 401 by strict ClaimsLayer
//! 4. Minted token carries service.internal permission and verifies via JwtVerifier
//!
//! ## What this covers (bd-8v2me)
//! Service A (shipping-receiving) mints a JWT with caller context. Service B
//! (any module behind ClaimsLayer) accepts and processes the request — the
//! tenant_id and actor_id from service A's JWT appear correctly in the
//! VerifiedClaims that service B's handler receives.

mod common;

use axum::{body::Body, extract::Extension, http::Request, routing::get, Router};
use base64::Engine as _;
use rsa::{
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
    RsaPrivateKey,
};
use security::{mint_service_jwt_with_context, ClaimsLayer, JwtVerifier, VerifiedClaims};
use std::sync::{Arc, OnceLock};
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Test RSA keypair — generated once per test process, stored in env var so
// that `mint_service_jwt_with_context` (which reads JWT_PRIVATE_KEY_PEM) works.
// ============================================================================

struct ServiceTestKeys {
    private_pem: String,
    public_pem: String,
}

fn service_test_keys() -> &'static ServiceTestKeys {
    static KEYS: OnceLock<ServiceTestKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate test RSA key");
        let pub_key = priv_key.to_public_key();
        let private_pem = priv_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("encode private key to PEM")
            .to_string();
        let public_pem = pub_key
            .to_public_key_pem(LineEnding::LF)
            .expect("encode public key to PEM");

        // Set env vars so mint_service_jwt_with_context can find the key.
        std::env::set_var("JWT_PRIVATE_KEY_PEM", &private_pem);
        std::env::set_var("SERVICE_NAME", "service.shipping-receiving");

        ServiceTestKeys {
            private_pem,
            public_pem,
        }
    })
}

/// Decode the payload section of a JWT without signature verification.
/// Used to inspect raw claim values before involving JwtVerifier.
fn decode_jwt_payload(token: &str) -> serde_json::Value {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3, "JWT must have 3 parts (header.payload.sig)");
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("base64-decode JWT payload");
    serde_json::from_slice(&payload_bytes).expect("JSON-parse JWT payload")
}

/// Minimal handler: returns the verified claims as JSON so the test can
/// assert that tenant_id and actor_id propagated through ClaimsLayer.
async fn echo_claims_handler(
    Extension(claims): Extension<VerifiedClaims>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "tenant_id": claims.tenant_id.to_string(),
        "user_id":   claims.user_id.to_string(),
        "perms":     claims.perms,
    }))
}

/// Build an in-process Axum router that acts as "service B":
/// one protected endpoint behind ClaimsLayer (strict mode).
fn build_protected_router() -> Router {
    let keys = service_test_keys();
    let verifier =
        Arc::new(JwtVerifier::from_public_pem(&keys.public_pem).expect("build JwtVerifier"));
    Router::new()
        .route("/protected", get(echo_claims_handler))
        .layer(ClaimsLayer::strict(verifier))
}

// ============================================================================
// Tests
// ============================================================================

/// Service A mints a JWT that contains tenant_id and actor_id.
///
/// Decodes the raw JWT payload and asserts both fields are present and
/// correct, independent of any verifier logic.
#[tokio::test]
async fn service_jwt_contains_tenant_id_and_actor_id() {
    service_test_keys(); // ensure env vars are set

    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let token = mint_service_jwt_with_context(tenant_id, actor_id)
        .expect("mint_service_jwt_with_context must succeed when JWT_PRIVATE_KEY_PEM is set");

    let payload = decode_jwt_payload(&token);

    assert_eq!(
        payload["tenant_id"].as_str().unwrap(),
        tenant_id.to_string(),
        "JWT payload must embed the caller tenant_id"
    );
    assert_eq!(
        payload["sub"].as_str().unwrap(),
        actor_id.to_string(),
        "JWT subject (sub) must be the actor_id"
    );
    assert!(
        payload["perms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str() == Some("service.internal")),
        "JWT must include service.internal in perms"
    );

    println!(
        "✅ service_jwt_contains_tenant_id_and_actor_id: tenant_id={tenant_id}, actor_id={actor_id}"
    );
}

/// Service B (strict ClaimsLayer) accepts a valid service JWT from service A
/// and propagates tenant_id correctly into VerifiedClaims.
#[tokio::test]
async fn service_jwt_accepted_by_claims_layer() {
    service_test_keys();

    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let token = mint_service_jwt_with_context(tenant_id, actor_id)
        .expect("mint_service_jwt_with_context failed");

    let app = build_protected_router();

    let request = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status().as_u16();

    assert_eq!(
        status, 200,
        "Service B must accept a valid service JWT (got {status})"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        body["tenant_id"].as_str().unwrap(),
        tenant_id.to_string(),
        "Service B must see the caller's tenant_id in VerifiedClaims"
    );
    assert_eq!(
        body["user_id"].as_str().unwrap(),
        actor_id.to_string(),
        "Service B must see the caller's actor_id as user_id in VerifiedClaims"
    );

    println!(
        "✅ service_jwt_accepted_by_claims_layer: tenant_id={tenant_id} propagated through ClaimsLayer"
    );
}

/// Service B rejects a request with no Authorization header (strict mode).
#[tokio::test]
async fn missing_token_rejected_by_strict_claims_layer() {
    service_test_keys();

    let app = build_protected_router();

    let request = Request::builder()
        .uri("/protected")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(
        response.status().as_u16(),
        401,
        "Request with no Bearer token must be rejected with 401 in strict mode"
    );

    println!("✅ missing_token_rejected_by_strict_claims_layer");
}

/// `JwtVerifier` decodes the minted token and exposes service.internal,
/// tenant_id, and actor_id through `VerifiedClaims`.
#[tokio::test]
async fn service_jwt_verified_claims_have_correct_context() {
    let keys = service_test_keys();
    let verifier = JwtVerifier::from_public_pem(&keys.public_pem).expect("build JwtVerifier");

    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let token = mint_service_jwt_with_context(tenant_id, actor_id)
        .expect("mint_service_jwt_with_context failed");

    let claims = verifier
        .verify(&token)
        .expect("JwtVerifier must accept a freshly minted service JWT");

    assert!(
        claims.perms.iter().any(|p| p == "service.internal"),
        "VerifiedClaims must include service.internal; got {:?}",
        claims.perms
    );
    assert_eq!(
        claims.tenant_id, tenant_id,
        "VerifiedClaims.tenant_id must match the caller's tenant_id"
    );
    assert_eq!(
        claims.user_id, actor_id,
        "VerifiedClaims.user_id must be the caller's actor_id"
    );

    println!(
        "✅ service_jwt_verified_claims_have_correct_context: perms={:?}, tenant={tenant_id}",
        claims.perms
    );
}

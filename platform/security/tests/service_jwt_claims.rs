//! Integration tests proving that mint_service_jwt_with_context embeds
//! tenant_id and actor_id in the minted JWT and that the token passes
//! JwtVerifier::verify() — the same path a receiving service's ClaimsLayer uses.
//!
//! Uses real RSA key generation; no mocks.

use std::sync::Mutex;
use uuid::Uuid;

use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;

use security::claims::{ActorType, JwtVerifier};
use security::service_auth::mint_service_jwt_with_context;

// Serialize env-mutating tests to avoid interference between parallel test threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct RsaTestKeys {
    private_pem: String,
    public_pem: String,
}

fn generate_rsa_keys() -> RsaTestKeys {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key generation");
    let pub_key = priv_key.to_public_key();
    let private_pem = priv_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("private PEM")
        .to_string();
    let public_pem = pub_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public PEM");
    RsaTestKeys {
        private_pem,
        public_pem,
    }
}

/// Core invariant: minted token must contain the exact tenant_id and actor_id
/// supplied by the caller, and must pass JwtVerifier (ClaimsLayer path).
#[test]
fn minted_token_embeds_tenant_and_actor() {
    let _guard = ENV_LOCK.lock().unwrap();
    let keys = generate_rsa_keys();
    std::env::set_var("JWT_PRIVATE_KEY_PEM", &keys.private_pem);
    std::env::set_var("SERVICE_NAME", "shipping-receiving");

    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let token = mint_service_jwt_with_context(tenant_id, actor_id)
        .expect("mint_service_jwt_with_context must succeed when JWT_PRIVATE_KEY_PEM is set");

    let verifier = JwtVerifier::from_public_pem(&keys.public_pem).expect("verifier");
    let claims = verifier
        .verify(&token)
        .expect("minted token must pass JwtVerifier — same path ClaimsLayer uses");

    assert_eq!(claims.tenant_id, tenant_id, "tenant_id must be forwarded");
    assert_eq!(claims.user_id, actor_id, "actor_id must be forwarded as sub/user_id");
    assert!(
        claims.perms.iter().any(|p| p == "service.internal"),
        "service.internal permission must be present"
    );
    assert_eq!(claims.actor_type, ActorType::Service);

    std::env::remove_var("JWT_PRIVATE_KEY_PEM");
    std::env::remove_var("SERVICE_NAME");
}

/// Nil UUIDs: get_service_token() still works (backward compat for background tasks
/// that have no request context). The token is valid but carries nil tenant/actor.
#[test]
fn no_context_token_is_still_valid() {
    let _guard = ENV_LOCK.lock().unwrap();
    let keys = generate_rsa_keys();
    std::env::set_var("JWT_PRIVATE_KEY_PEM", &keys.private_pem);
    std::env::remove_var("SERVICE_TOKEN"); // force fresh mint

    let token = security::service_auth::get_service_token()
        .expect("get_service_token must succeed when JWT_PRIVATE_KEY_PEM is set");

    let verifier = JwtVerifier::from_public_pem(&keys.public_pem).expect("verifier");
    let claims = verifier.verify(&token).expect("no-context token must be verifiable");

    assert_eq!(claims.tenant_id, Uuid::nil(), "no-context token uses nil tenant");
    assert_eq!(claims.user_id, Uuid::nil(), "no-context token uses nil actor");
    assert!(claims.perms.iter().any(|p| p == "service.internal"));

    std::env::remove_var("JWT_PRIVATE_KEY_PEM");
}

/// Missing private key returns MissingSigningKey, not a panic.
#[test]
fn missing_private_key_returns_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("JWT_PRIVATE_KEY_PEM");

    let result = mint_service_jwt_with_context(Uuid::new_v4(), Uuid::new_v4());
    assert!(
        matches!(result, Err(security::service_auth::ServiceAuthError::MissingSigningKey)),
        "expected MissingSigningKey, got: {result:?}"
    );
}

/// Different callers get tokens with their own distinct tenant/actor claims.
#[test]
fn context_is_caller_specific() {
    let _guard = ENV_LOCK.lock().unwrap();
    let keys = generate_rsa_keys();
    std::env::set_var("JWT_PRIVATE_KEY_PEM", &keys.private_pem);

    let tenant_a = Uuid::new_v4();
    let actor_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let actor_b = Uuid::new_v4();

    let token_a = mint_service_jwt_with_context(tenant_a, actor_a).expect("mint A");
    let token_b = mint_service_jwt_with_context(tenant_b, actor_b).expect("mint B");

    let verifier = JwtVerifier::from_public_pem(&keys.public_pem).expect("verifier");

    let claims_a = verifier.verify(&token_a).expect("verify A");
    let claims_b = verifier.verify(&token_b).expect("verify B");

    assert_eq!(claims_a.tenant_id, tenant_a);
    assert_eq!(claims_a.user_id, actor_a);
    assert_eq!(claims_b.tenant_id, tenant_b);
    assert_eq!(claims_b.user_id, actor_b);
    assert_ne!(claims_a.tenant_id, claims_b.tenant_id);

    std::env::remove_var("JWT_PRIVATE_KEY_PEM");
}

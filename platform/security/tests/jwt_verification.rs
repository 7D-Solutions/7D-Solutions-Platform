//! Integration tests for JWT RS256 verification.
//!
//! These tests generate real RSA keypairs and sign JWTs to exercise
//! the full verification pipeline — no mocks or stubs.

use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serde::Serialize;
use uuid::Uuid;

use security::claims::{ActorType, JwtVerifier};
use security::SecurityError;

// ── Helpers ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

struct KeyPair {
    encoding: EncodingKey,
    pub_pem: String,
}

fn gen_keypair() -> KeyPair {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
    KeyPair {
        encoding: EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap(),
        pub_pem,
    }
}

fn sign(enc: &EncodingKey, claims: &TestClaims) -> String {
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), claims, enc).unwrap()
}

fn valid_claims() -> TestClaims {
    let now = Utc::now();
    TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: Uuid::new_v4().to_string(),
        app_id: None,
        roles: vec!["admin".into()],
        perms: vec!["ar.mutate".into(), "gl.post".into()],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    }
}

// ── Valid token ──────────────────────────────────────────────────────────

#[test]
fn valid_token_returns_correct_claims() {
    let kp = gen_keypair();
    let claims = valid_claims();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    let v = verifier.verify(&token).unwrap();

    assert_eq!(v.user_id.to_string(), claims.sub);
    assert_eq!(v.tenant_id.to_string(), claims.tenant_id);
    assert_eq!(v.roles, vec!["admin"]);
    assert_eq!(v.perms, vec!["ar.mutate", "gl.post"]);
    assert_eq!(v.actor_type, ActorType::User);
    assert!(v.app_id.is_none());
}

#[test]
fn valid_token_with_app_id() {
    let kp = gen_keypair();
    let app = Uuid::new_v4();
    let mut claims = valid_claims();
    claims.app_id = Some(app.to_string());
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    let v = verifier.verify(&token).unwrap();
    assert_eq!(v.app_id, Some(app));
}

#[test]
fn valid_token_service_actor() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.actor_type = "service".to_string();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert_eq!(
        verifier.verify(&token).unwrap().actor_type,
        ActorType::Service
    );
}

#[test]
fn valid_token_system_actor() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.actor_type = "system".to_string();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert_eq!(
        verifier.verify(&token).unwrap().actor_type,
        ActorType::System
    );
}

// ── Expired token ────────────────────────────────────────────────────────

#[test]
fn expired_token_returns_token_expired() {
    let kp = gen_keypair();
    let now = Utc::now();
    let mut claims = valid_claims();
    claims.iat = (now - Duration::hours(1)).timestamp();
    claims.exp = (now - Duration::minutes(5)).timestamp();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::TokenExpired)
    ));
}

// ── Invalid tokens ───────────────────────────────────────────────────────

#[test]
fn wrong_signing_key_rejected() {
    let kp_sign = gen_keypair();
    let kp_verify = gen_keypair();
    let token = sign(&kp_sign.encoding, &valid_claims());

    let verifier = JwtVerifier::from_public_pem(&kp_verify.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn garbage_token_rejected() {
    let kp = gen_keypair();
    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify("not.a.jwt"),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn empty_token_rejected() {
    let kp = gen_keypair();
    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(""),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn wrong_issuer_rejected() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.iss = "evil-issuer".to_string();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn wrong_audience_rejected() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.aud = "wrong-audience".to_string();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn invalid_uuid_in_sub_rejected() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.sub = "not-a-uuid".to_string();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn invalid_actor_type_rejected() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.actor_type = "unknown".to_string();
    let token = sign(&kp.encoding, &claims);

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

// ── Key rotation ─────────────────────────────────────────────────────────

#[test]
fn rotation_overlap_accepts_old_key_token() {
    let old_kp = gen_keypair();
    let new_kp = gen_keypair();
    let token = sign(&old_kp.encoding, &valid_claims());

    let mut verifier = JwtVerifier::from_public_pem(&new_kp.pub_pem).unwrap();
    verifier.with_prev_key(&old_kp.pub_pem).unwrap();

    assert_eq!(verifier.verify(&token).unwrap().actor_type, ActorType::User);
}

#[test]
fn rotation_overlap_accepts_new_key_token() {
    let old_kp = gen_keypair();
    let new_kp = gen_keypair();
    let token = sign(&new_kp.encoding, &valid_claims());

    let mut verifier = JwtVerifier::from_public_pem(&new_kp.pub_pem).unwrap();
    verifier.with_prev_key(&old_kp.pub_pem).unwrap();

    assert_eq!(verifier.verify(&token).unwrap().actor_type, ActorType::User);
}

#[test]
fn rotation_cutover_accepts_both_keys_then_rejects_old_token() {
    let key_a = gen_keypair();
    let key_b = gen_keypair();
    let claims = valid_claims();
    let token_a = sign(&key_a.encoding, &claims);
    let token_b = sign(&key_b.encoding, &claims);

    let mut overlap_verifier = JwtVerifier::from_public_pem(&key_b.pub_pem).unwrap();
    overlap_verifier.with_prev_key(&key_a.pub_pem).unwrap();

    assert_eq!(
        overlap_verifier.verify(&token_a).unwrap().actor_type,
        ActorType::User
    );
    assert_eq!(
        overlap_verifier.verify(&token_b).unwrap().actor_type,
        ActorType::User
    );

    let cutover_verifier = JwtVerifier::from_public_pem(&key_b.pub_pem).unwrap();
    assert!(matches!(
        cutover_verifier.verify(&token_a),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn rotation_ended_old_key_rejected() {
    let old_kp = gen_keypair();
    let new_kp = gen_keypair();
    let token = sign(&old_kp.encoding, &valid_claims());

    let verifier = JwtVerifier::from_public_pem(&new_kp.pub_pem).unwrap();
    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

#[test]
fn rotation_third_party_key_rejected() {
    let old_kp = gen_keypair();
    let new_kp = gen_keypair();
    let rogue_kp = gen_keypair();
    let token = sign(&rogue_kp.encoding, &valid_claims());

    let mut verifier = JwtVerifier::from_public_pem(&new_kp.pub_pem).unwrap();
    verifier.with_prev_key(&old_kp.pub_pem).unwrap();

    assert!(matches!(
        verifier.verify(&token),
        Err(SecurityError::InvalidToken)
    ));
}

// ── Verifier construction ────────────────────────────────────────────────

#[test]
fn invalid_pem_returns_error() {
    assert!(JwtVerifier::from_public_pem("not a pem").is_err());
}

#[test]
fn invalid_prev_pem_returns_error() {
    let kp = gen_keypair();
    let mut verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert!(verifier.with_prev_key("garbage").is_err());
}

// ── Edge cases ───────────────────────────────────────────────────────────

#[test]
fn token_carries_many_permissions() {
    let kp = gen_keypair();
    let mut claims = valid_claims();
    claims.perms = vec![
        "ar.mutate".into(),
        "gl.post".into(),
        "inventory.read".into(),
        "treasury.mutate".into(),
    ];
    let token = sign(&kp.encoding, &claims);

    let v = JwtVerifier::from_public_pem(&kp.pub_pem)
        .unwrap()
        .verify(&token)
        .unwrap();
    assert_eq!(v.perms.len(), 4);
    assert!(v.perms.contains(&"treasury.mutate".to_string()));
}

#[test]
fn different_tokens_have_different_jti() {
    let kp = gen_keypair();
    let t1 = sign(&kp.encoding, &valid_claims());
    let t2 = sign(&kp.encoding, &valid_claims());

    let verifier = JwtVerifier::from_public_pem(&kp.pub_pem).unwrap();
    assert_ne!(
        verifier.verify(&t1).unwrap().token_id,
        verifier.verify(&t2).unwrap().token_id
    );
}

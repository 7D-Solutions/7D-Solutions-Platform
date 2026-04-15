use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use security::{ClaimsLayer, JwtVerifier};
use serde::Serialize;
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

#[derive(Serialize)]
pub struct TestClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    pub tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
    pub actor_type: String,
    pub ver: String,
}

pub struct TestJwtKeys {
    pub encoding: EncodingKey,
    pub verifier: Arc<JwtVerifier>,
}

pub fn test_jwt_keys() -> &'static TestJwtKeys {
    static KEYS: OnceLock<TestJwtKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key gen");
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("PEM encode");
        let pub_pem = pub_key
            .to_public_key_pem(LineEnding::LF)
            .expect("public PEM");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
        let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).expect("JWT verifier"));
        TestJwtKeys { encoding, verifier }
    })
}

pub fn sign_test_jwt(tenant_id: &str, perms: &[&str]) -> String {
    let keys = test_jwt_keys();
    let now = chrono::Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        app_id: Some(tenant_id.to_string()),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &keys.encoding)
        .expect("JWT signing failed")
}

pub fn with_test_jwt_layer(router: axum::Router) -> axum::Router {
    let keys = test_jwt_keys();
    router.layer(ClaimsLayer::permissive(keys.verifier.clone()))
}

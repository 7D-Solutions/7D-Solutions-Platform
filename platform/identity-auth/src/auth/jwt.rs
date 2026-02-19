use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// JWKS support
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPublicKey;

/// Current claims schema version. Bump when adding/removing fields.
pub const CLAIMS_VERSION: &str = "1";

/// Actor type constants — aligned with EventEnvelope `actor_type`.
pub mod actor_type {
    pub const USER: &str = "user";
    pub const SERVICE: &str = "service";
    pub const SYSTEM: &str = "system";
}

/// Platform access token claims (version 1).
///
/// Canonical JWT payload issued by identity-auth. The `ver` field enables
/// schema evolution without breaking existing verifiers.
///
/// Alignment with EventEnvelope:
/// - `sub` (user_id) → `actor_id`
/// - `actor_type` → `actor_type`
/// - `tenant_id` → `tenant_id`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessClaims {
    // ── Standard JWT (RFC 7519) ──
    pub sub: String,        // user_id (UUID string)
    pub iss: String,        // issuer ("auth-rs")
    pub aud: String,        // audience ("7d-platform")
    pub iat: i64,           // issued at (Unix timestamp)
    pub exp: i64,           // expires at (Unix timestamp)
    pub jti: String,        // unique token ID (UUID)

    // ── Platform identity ──
    pub tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
    pub actor_type: String,

    // ── Versioning ──
    pub ver: String,
}

#[derive(Debug, Serialize)]
pub struct Jwks {
    pub keys: Vec<JwkKey>,
}

#[derive(Debug, Serialize)]
pub struct JwkKey {
    pub kty: &'static str, // "RSA"
    #[serde(rename = "use")]
    pub use_: &'static str, // "sig"
    pub kid: String,
    pub alg: &'static str, // "RS256"
    pub n: String,         // base64url(modulus)
    pub e: String,         // base64url(exponent)
}

#[derive(Clone)]
pub struct JwtKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    pub kid: String,

    // IMPORTANT: store the public PEM so we can serve JWKS
    pub public_key_pem: String,
}

impl JwtKeys {
    pub fn from_pem(private_pem: &str, public_pem: &str, kid: String) -> Result<Self, String> {
        let encoding =
            EncodingKey::from_rsa_pem(private_pem.as_bytes()).map_err(|e| e.to_string())?;
        let decoding =
            DecodingKey::from_rsa_pem(public_pem.as_bytes()).map_err(|e| e.to_string())?;

        Ok(Self {
            encoding,
            decoding,
            kid,
            public_key_pem: public_pem.to_string(),
        })
    }

    // ---------- getters for wiring ----------
    pub fn kid(&self) -> &str {
        &self.kid
    }

    pub fn public_key_pem(&self) -> &str {
        &self.public_key_pem
    }

    // ---------- JWT ----------
    pub fn sign_access_token(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
        roles: Vec<String>,
        perms: Vec<String>,
        actor_type: &str,
        ttl_minutes: i64,
    ) -> Result<String, String> {
        let now = Utc::now();
        let exp = now + Duration::minutes(ttl_minutes);

        let claims = AccessClaims {
            sub: user_id.to_string(),
            tenant_id: tenant_id.to_string(),
            app_id: None,
            iss: "auth-rs".to_string(),
            aud: "7d-platform".to_string(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
            jti: Uuid::new_v4().to_string(),
            roles,
            perms,
            actor_type: actor_type.to_string(),
            ver: CLAIMS_VERSION.to_string(),
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());

        jsonwebtoken::encode(&header, &claims, &self.encoding).map_err(|e| e.to_string())
    }

    pub fn validate_access_token(&self, token: &str) -> Result<AccessClaims, String> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;

        // Validate issuer and audience to prevent cross-environment/service token reuse
        validation.set_issuer(&["auth-rs"]);
        validation.set_audience(&["7d-platform"]);

        let data = jsonwebtoken::decode::<AccessClaims>(token, &self.decoding, &validation)
            .map_err(|e| e.to_string())?;

        Ok(data.claims)
    }

    // ---------- JWKS ----------
    pub fn to_jwks(&self) -> Result<Jwks, String> {
        // Your public key is likely "BEGIN PUBLIC KEY" (SPKI/PKCS#8)
        let pubkey =
            RsaPublicKey::from_public_key_pem(self.public_key_pem()).map_err(|e| e.to_string())?;

        let n_bytes = pubkey.n().to_bytes_be();
        let e_bytes = pubkey.e().to_bytes_be();

        let n = URL_SAFE_NO_PAD.encode(n_bytes);
        let e = URL_SAFE_NO_PAD.encode(e_bytes);

        Ok(Jwks {
            keys: vec![JwkKey {
                kty: "RSA",
                use_: "sig",
                kid: self.kid.clone(),
                alg: "RS256",
                n,
                e,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keys() -> JwtKeys {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::RsaPrivateKey;

        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = private_key.to_public_key();

        let priv_pem = private_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let pub_pem = public_key.to_public_key_pem(LineEnding::LF).unwrap();

        JwtKeys::from_pem(&priv_pem, &pub_pem, "test-kid".to_string()).unwrap()
    }

    #[test]
    fn claims_round_trip() {
        let keys = test_keys();
        let tenant = Uuid::new_v4();
        let user = Uuid::new_v4();
        let roles = vec!["admin".to_string(), "operator".to_string()];
        let perms = vec!["ar.create".to_string(), "gl.post".to_string()];

        let token = keys
            .sign_access_token(tenant, user, roles.clone(), perms.clone(), actor_type::USER, 15)
            .unwrap();

        let decoded = keys.validate_access_token(&token).unwrap();

        assert_eq!(decoded.sub, user.to_string());
        assert_eq!(decoded.tenant_id, tenant.to_string());
        assert_eq!(decoded.roles, roles);
        assert_eq!(decoded.perms, perms);
        assert_eq!(decoded.actor_type, "user");
        assert_eq!(decoded.ver, CLAIMS_VERSION);
        assert_eq!(decoded.iss, "auth-rs");
        assert_eq!(decoded.aud, "7d-platform");
        assert!(decoded.app_id.is_none());
    }

    #[test]
    fn claims_version_is_set() {
        assert_eq!(CLAIMS_VERSION, "1");
    }

    #[test]
    fn claims_service_actor_type() {
        let keys = test_keys();
        let token = keys
            .sign_access_token(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                actor_type::SERVICE,
                5,
            )
            .unwrap();

        let decoded = keys.validate_access_token(&token).unwrap();
        assert_eq!(decoded.actor_type, "service");
    }

    #[test]
    fn claims_empty_roles_perms() {
        let keys = test_keys();
        let token = keys
            .sign_access_token(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                actor_type::USER,
                5,
            )
            .unwrap();

        let decoded = keys.validate_access_token(&token).unwrap();
        assert!(decoded.roles.is_empty());
        assert!(decoded.perms.is_empty());
    }

    #[test]
    fn claims_expired_token_rejected() {
        let keys = test_keys();
        // TTL of 0 minutes — token expires immediately
        let token = keys
            .sign_access_token(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                actor_type::USER,
                0,
            )
            .unwrap();

        // jsonwebtoken has a leeway of 0 by default, but iat == exp should fail
        // We need a truly expired token: use -1 minute hack via direct construction
        let now = Utc::now();
        let claims = AccessClaims {
            sub: Uuid::new_v4().to_string(),
            iss: "auth-rs".to_string(),
            aud: "7d-platform".to_string(),
            iat: (now - Duration::minutes(10)).timestamp(),
            exp: (now - Duration::minutes(5)).timestamp(),
            jti: Uuid::new_v4().to_string(),
            tenant_id: Uuid::new_v4().to_string(),
            app_id: None,
            roles: vec![],
            perms: vec![],
            actor_type: "user".to_string(),
            ver: "1".to_string(),
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".to_string());
        let expired_token =
            jsonwebtoken::encode(&header, &claims, &keys.encoding).unwrap();

        let result = keys.validate_access_token(&expired_token);
        assert!(result.is_err());
        // Drop the zero-TTL token test — it may pass due to timing
        let _ = token;
    }
}

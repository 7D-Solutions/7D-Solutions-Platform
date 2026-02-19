//! JWT claims verification for platform access tokens.
//!
//! Verifies RS256-signed JWTs issued by identity-auth and returns structured
//! [`VerifiedClaims`] for downstream service consumption.

use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::SecurityError;

/// Actor types aligned with EventEnvelope `actor_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorType {
    User,
    Service,
    System,
}

impl ActorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActorType::User => "user",
            ActorType::Service => "service",
            ActorType::System => "system",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(ActorType::User),
            "service" => Some(ActorType::Service),
            "system" => Some(ActorType::System),
            _ => None,
        }
    }
}

/// Verified JWT claims returned after successful token validation.
///
/// All string UUIDs from the raw JWT are parsed into typed [`Uuid`] values.
/// Downstream services should use this struct — never decode tokens manually.
#[derive(Debug, Clone)]
pub struct VerifiedClaims {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub app_id: Option<Uuid>,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
    pub actor_type: ActorType,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub token_id: Uuid,
    pub version: String,
}

/// Raw JWT payload (deserialization target matching identity-auth AccessClaims).
#[derive(Debug, Deserialize)]
struct RawAccessClaims {
    pub sub: String,
    #[allow(dead_code)]
    pub iss: String,
    #[allow(dead_code)]
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    pub tenant_id: String,
    pub app_id: Option<String>,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
    pub actor_type: String,
    pub ver: String,
}

/// JWT verifier for platform access tokens.
///
/// Holds the RSA public key and validation rules. Create once at service
/// startup, then call [`verify`](JwtVerifier::verify) on each request.
#[derive(Clone)]
pub struct JwtVerifier {
    decoding: DecodingKey,
    validation: Validation,
}

impl JwtVerifier {
    /// Create a verifier from an RSA public key PEM.
    pub fn from_public_pem(pem: &str) -> Result<Self, String> {
        let decoding = DecodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| format!("invalid public key: {e}"))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;
        validation.set_issuer(&["auth-rs"]);
        validation.set_audience(&["7d-platform"]);

        Ok(Self {
            decoding,
            validation,
        })
    }

    /// Create a verifier from the `JWT_PUBLIC_KEY` environment variable.
    ///
    /// Returns `None` when the variable is absent (e.g. in local dev environments
    /// that have not yet configured an identity service).  Services should still
    /// mount [`RequirePermissionsLayer`](crate::authz_middleware::RequirePermissionsLayer)
    /// on mutation routes — when no `JwtVerifier` is provided, no claims will be
    /// extracted and those routes will respond **401 Unauthorized**.
    pub fn from_env() -> Option<Self> {
        let pem = std::env::var("JWT_PUBLIC_KEY").ok()?;
        Self::from_public_pem(&pem)
            .map_err(|e| tracing::warn!("JWT_PUBLIC_KEY is set but invalid: {}", e))
            .ok()
    }

    /// Verify a Bearer token and return structured claims.
    pub fn verify(&self, token: &str) -> Result<VerifiedClaims, SecurityError> {
        let data =
            jsonwebtoken::decode::<RawAccessClaims>(token, &self.decoding, &self.validation)
                .map_err(|e| match e.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                        SecurityError::TokenExpired
                    }
                    _ => SecurityError::InvalidToken,
                })?;

        let raw = data.claims;
        Self::convert_raw(raw)
    }

    fn convert_raw(raw: RawAccessClaims) -> Result<VerifiedClaims, SecurityError> {
        let user_id =
            Uuid::parse_str(&raw.sub).map_err(|_| SecurityError::InvalidToken)?;
        let tenant_id =
            Uuid::parse_str(&raw.tenant_id).map_err(|_| SecurityError::InvalidToken)?;
        let app_id = raw
            .app_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| SecurityError::InvalidToken)?;
        let token_id =
            Uuid::parse_str(&raw.jti).map_err(|_| SecurityError::InvalidToken)?;
        let actor_type =
            ActorType::from_str(&raw.actor_type).ok_or(SecurityError::InvalidToken)?;
        let issued_at =
            DateTime::from_timestamp(raw.iat, 0).ok_or(SecurityError::InvalidToken)?;
        let expires_at =
            DateTime::from_timestamp(raw.exp, 0).ok_or(SecurityError::InvalidToken)?;

        Ok(VerifiedClaims {
            user_id,
            tenant_id,
            app_id,
            roles: raw.roles,
            perms: raw.perms,
            actor_type,
            issued_at,
            expires_at,
            token_id,
            version: raw.ver,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::RsaPrivateKey;
    use serde::Serialize;

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

    fn make_keys() -> (EncodingKey, String) {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
        let enc = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap();
        (enc, pub_pem)
    }

    fn sign_test_token(enc: &EncodingKey, claims: &TestClaims) -> String {
        let header = Header::new(Algorithm::RS256);
        jsonwebtoken::encode(&header, claims, enc).unwrap()
    }

    fn default_claims() -> TestClaims {
        let now = Utc::now();
        TestClaims {
            sub: Uuid::new_v4().to_string(),
            iss: "auth-rs".to_string(),
            aud: "7d-platform".to_string(),
            iat: now.timestamp(),
            exp: (now + chrono::Duration::minutes(15)).timestamp(),
            jti: Uuid::new_v4().to_string(),
            tenant_id: Uuid::new_v4().to_string(),
            app_id: None,
            roles: vec!["admin".into()],
            perms: vec!["ar.create".into(), "gl.post".into()],
            actor_type: "user".to_string(),
            ver: "1".to_string(),
        }
    }

    #[test]
    fn verify_valid_token() {
        let (enc, pub_pem) = make_keys();
        let claims = default_claims();
        let token = sign_test_token(&enc, &claims);

        let verifier = JwtVerifier::from_public_pem(&pub_pem).unwrap();
        let verified = verifier.verify(&token).unwrap();

        assert_eq!(verified.user_id.to_string(), claims.sub);
        assert_eq!(verified.tenant_id.to_string(), claims.tenant_id);
        assert_eq!(verified.roles, vec!["admin"]);
        assert_eq!(verified.perms, vec!["ar.create", "gl.post"]);
        assert_eq!(verified.actor_type, ActorType::User);
        assert_eq!(verified.version, "1");
        assert!(verified.app_id.is_none());
    }

    #[test]
    fn verify_expired_token() {
        let (enc, pub_pem) = make_keys();
        let now = Utc::now();
        let mut claims = default_claims();
        claims.iat = (now - chrono::Duration::minutes(20)).timestamp();
        claims.exp = (now - chrono::Duration::minutes(5)).timestamp();
        let token = sign_test_token(&enc, &claims);

        let verifier = JwtVerifier::from_public_pem(&pub_pem).unwrap();
        let result = verifier.verify(&token);
        assert!(matches!(result, Err(SecurityError::TokenExpired)));
    }

    #[test]
    fn verify_wrong_key_rejected() {
        let (enc, _) = make_keys();
        let (_, other_pub_pem) = make_keys();
        let claims = default_claims();
        let token = sign_test_token(&enc, &claims);

        let verifier = JwtVerifier::from_public_pem(&other_pub_pem).unwrap();
        let result = verifier.verify(&token);
        assert!(matches!(result, Err(SecurityError::InvalidToken)));
    }

    #[test]
    fn verify_with_app_id() {
        let (enc, pub_pem) = make_keys();
        let app = Uuid::new_v4();
        let mut claims = default_claims();
        claims.app_id = Some(app.to_string());
        let token = sign_test_token(&enc, &claims);

        let verifier = JwtVerifier::from_public_pem(&pub_pem).unwrap();
        let verified = verifier.verify(&token).unwrap();
        assert_eq!(verified.app_id, Some(app));
    }

    #[test]
    fn actor_type_roundtrip() {
        assert_eq!(ActorType::from_str("user"), Some(ActorType::User));
        assert_eq!(ActorType::from_str("service"), Some(ActorType::Service));
        assert_eq!(ActorType::from_str("system"), Some(ActorType::System));
        assert_eq!(ActorType::from_str("bogus"), None);
        assert_eq!(ActorType::User.as_str(), "user");
        assert_eq!(ActorType::Service.as_str(), "service");
        assert_eq!(ActorType::System.as_str(), "system");
    }
}

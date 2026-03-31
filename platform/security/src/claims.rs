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

    #[allow(clippy::should_implement_trait)]
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
    // Previous key present during zero-downtime JWT key rotation overlap.
    prev_decoding: Option<DecodingKey>,
}

impl JwtVerifier {
    /// Create a verifier from an RSA public key PEM.
    pub fn from_public_pem(pem: &str) -> Result<Self, String> {
        let decoding = DecodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| format!("invalid public key: {e}"))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;
        // Allow 30 seconds of clock skew between identity service and consumers.
        // Without this, even minor clock drift causes valid tokens to be rejected
        // at the boundary of their lifetime.
        validation.leeway = 30;
        validation.set_issuer(&["auth-rs"]);
        validation.set_audience(&["7d-platform"]);

        Ok(Self {
            decoding,
            validation,
            prev_decoding: None,
        })
    }

    /// Attach a previous (retiring) public key for zero-downtime rotation.
    ///
    /// During the rotation overlap window, tokens signed by either key are
    /// accepted. Remove `JWT_PUBLIC_KEY_PREV` once all outstanding tokens
    /// signed by the old key have expired (typically after one TTL cycle).
    pub fn with_prev_key(&mut self, pem: &str) -> Result<(), String> {
        self.prev_decoding = Some(
            DecodingKey::from_rsa_pem(pem.as_bytes())
                .map_err(|e| format!("invalid prev public key: {e}"))?,
        );
        Ok(())
    }

    /// Create a verifier from the `JWT_PUBLIC_KEY` environment variable.
    /// Falls back to `JWT_PUBLIC_KEY_PEM` if `JWT_PUBLIC_KEY` is not set.
    ///
    /// Returns `None` when neither variable is present (e.g. in local dev
    /// environments that have not yet configured an identity service).  Services
    /// should still mount [`RequirePermissionsLayer`](crate::authz_middleware::RequirePermissionsLayer)
    /// on mutation routes — when no `JwtVerifier` is provided, no claims will be
    /// extracted and those routes will respond **401 Unauthorized**.
    pub fn from_env() -> Option<Self> {
        let pem = std::env::var("JWT_PUBLIC_KEY")
            .or_else(|_| std::env::var("JWT_PUBLIC_KEY_PEM"))
            .ok()?;
        Self::from_public_pem(&pem)
            .map_err(|e| tracing::warn!("JWT public key is set but invalid: {}", e))
            .ok()
    }

    /// Like [`from_env`] but also reads `JWT_PUBLIC_KEY_PREV` for the rotation
    /// overlap window. Use this constructor in all service startup paths so that
    /// key rotation requires only an env-var update + rolling restart.
    pub fn from_env_with_overlap() -> Option<Self> {
        let pem = std::env::var("JWT_PUBLIC_KEY")
            .or_else(|_| std::env::var("JWT_PUBLIC_KEY_PEM"))
            .ok()?;
        let mut verifier = Self::from_public_pem(&pem)
            .map_err(|e| tracing::warn!("JWT public key is set but invalid: {}", e))
            .ok()?;

        if let Ok(prev_pem) = std::env::var("JWT_PUBLIC_KEY_PREV") {
            if let Err(e) = verifier.with_prev_key(&prev_pem) {
                tracing::warn!("JWT_PUBLIC_KEY_PREV is set but invalid: {}", e);
            }
        }

        Some(verifier)
    }

    /// Verify a Bearer token and return structured claims.
    pub fn verify(&self, token: &str) -> Result<VerifiedClaims, SecurityError> {
        match jsonwebtoken::decode::<RawAccessClaims>(token, &self.decoding, &self.validation) {
            Ok(data) => Self::convert_raw(data.claims),
            Err(primary_err) => {
                // During rotation overlap: try the previous key before failing.
                if let Some(ref prev) = self.prev_decoding {
                    if let Ok(data) =
                        jsonwebtoken::decode::<RawAccessClaims>(token, prev, &self.validation)
                    {
                        return Self::convert_raw(data.claims);
                    }
                }
                Err(match primary_err.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                        SecurityError::TokenExpired
                    }
                    _ => SecurityError::InvalidToken,
                })
            }
        }
    }

    fn convert_raw(raw: RawAccessClaims) -> Result<VerifiedClaims, SecurityError> {
        let user_id = Uuid::parse_str(&raw.sub).map_err(|_| SecurityError::InvalidToken)?;
        let tenant_id = Uuid::parse_str(&raw.tenant_id).map_err(|_| SecurityError::InvalidToken)?;
        let app_id = raw
            .app_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| SecurityError::InvalidToken)?;
        let token_id = Uuid::parse_str(&raw.jti).map_err(|_| SecurityError::InvalidToken)?;
        let actor_type = ActorType::from_str(&raw.actor_type).ok_or(SecurityError::InvalidToken)?;
        let issued_at = DateTime::from_timestamp(raw.iat, 0).ok_or(SecurityError::InvalidToken)?;
        let expires_at = DateTime::from_timestamp(raw.exp, 0).ok_or(SecurityError::InvalidToken)?;

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
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key gen");
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("PEM encode");
        let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).expect("public PEM");
        let enc = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
        (enc, pub_pem)
    }

    fn sign_test_token(enc: &EncodingKey, claims: &TestClaims) -> String {
        let header = Header::new(Algorithm::RS256);
        jsonwebtoken::encode(&header, claims, enc).expect("sign token")
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

        let verifier = JwtVerifier::from_public_pem(&pub_pem).expect("verifier");
        let verified = verifier.verify(&token).expect("verify");

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

        let verifier = JwtVerifier::from_public_pem(&pub_pem).expect("verifier");
        let result = verifier.verify(&token);
        assert!(matches!(result, Err(SecurityError::TokenExpired)));
    }

    #[test]
    fn verify_wrong_key_rejected() {
        let (enc, _) = make_keys();
        let (_, other_pub_pem) = make_keys();
        let claims = default_claims();
        let token = sign_test_token(&enc, &claims);

        let verifier = JwtVerifier::from_public_pem(&other_pub_pem).expect("verifier");
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

        let verifier = JwtVerifier::from_public_pem(&pub_pem).expect("verifier");
        let verified = verifier.verify(&token).expect("verify");
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

    /// Zero-downtime JWT rotation: verifier configured with new primary key and
    /// old retiring key must accept tokens signed by either key during the overlap window.
    #[test]
    fn rotation_overlap_accepts_token_from_prev_key() {
        let (old_enc, old_pub_pem) = make_keys();
        let (_new_enc, new_pub_pem) = make_keys();

        // Token was issued before rotation — signed with the OLD key
        let old_claims = default_claims();
        let token_signed_with_old_key = sign_test_token(&old_enc, &old_claims);

        // New verifier uses new primary key + old key as prev (overlap window)
        let mut verifier = JwtVerifier::from_public_pem(&new_pub_pem).expect("verifier");
        verifier.with_prev_key(&old_pub_pem).expect("prev key");

        // Old token must still verify successfully during overlap
        let verified = verifier.verify(&token_signed_with_old_key).expect("verify");
        assert_eq!(verified.roles, vec!["admin"]);
        assert_eq!(verified.actor_type, ActorType::User);
    }

    /// After the overlap window ends (prev key cleared), tokens signed by the old
    /// key must be rejected.
    #[test]
    fn rotation_overlap_ends_old_token_rejected() {
        let (old_enc, _old_pub_pem) = make_keys();
        let (_new_enc, new_pub_pem) = make_keys();

        let old_claims = default_claims();
        let token_signed_with_old_key = sign_test_token(&old_enc, &old_claims);

        // Verifier with ONLY the new key (overlap has ended)
        let verifier = JwtVerifier::from_public_pem(&new_pub_pem).expect("verifier");

        // Old-key token must now be rejected
        assert!(matches!(
            verifier.verify(&token_signed_with_old_key),
            Err(SecurityError::InvalidToken)
        ));
    }
}

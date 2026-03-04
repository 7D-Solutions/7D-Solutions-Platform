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
pub const CLAIMS_VERSION: &str = "2";

/// Actor type constants — aligned with EventEnvelope `actor_type`.
#[allow(dead_code)]
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
    pub sub: String, // user_id (UUID string)
    pub iss: String, // issuer ("auth-rs")
    pub aud: String, // audience ("7d-platform")
    pub iat: i64,    // issued at (Unix timestamp)
    pub exp: i64,    // expires at (Unix timestamp)
    pub jti: String, // unique token ID (UUID)

    // ── Platform identity ──
    pub tenant_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
    pub actor_type: String,

    // ── Audit enrichment ──
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_snapshot_id: Option<String>,

    // ── Versioning ──
    pub ver: String,
}

/// Compute a deterministic snapshot ID from a sorted set of role names.
/// Used to detect stale tokens when role assignments change.
pub fn compute_role_snapshot_id(roles: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted = roles.to_vec();
    sorted.sort();
    let input = sorted.join(",");
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(&hash[..8]) // 16-char hex, compact but sufficient
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
#[allow(dead_code)]
pub struct JwtKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    pub kid: String,

    // IMPORTANT: store the public PEM so we can serve JWKS
    pub public_key_pem: String,

    // Previous key — present during zero-downtime rotation overlap window.
    // Tokens signed with the old key are still accepted until this is cleared.
    prev_decoding: Option<DecodingKey>,
    prev_kid: Option<String>,
    prev_public_key_pem: Option<String>,
}

#[allow(dead_code)]
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
            prev_decoding: None,
            prev_kid: None,
            prev_public_key_pem: None,
        })
    }

    /// Attach a previous (retiring) key for zero-downtime rotation.
    ///
    /// During the overlap window, tokens signed by either the current or the
    /// previous key are accepted. The previous public key is also included in
    /// the JWKS endpoint so that other verifiers can pick it up.
    ///
    /// Remove the previous key (and the env vars that set it) once all
    /// outstanding tokens signed by it have expired.
    pub fn with_prev_key(&mut self, prev_public_pem: &str, prev_kid: String) -> Result<(), String> {
        self.prev_decoding =
            Some(DecodingKey::from_rsa_pem(prev_public_pem.as_bytes()).map_err(|e| e.to_string())?);
        self.prev_kid = Some(prev_kid);
        self.prev_public_key_pem = Some(prev_public_pem.to_string());
        Ok(())
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
        self.sign_access_token_enriched(
            tenant_id, user_id, roles, perms, actor_type, ttl_minutes, None, None,
        )
    }

    pub fn sign_access_token_enriched(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
        roles: Vec<String>,
        perms: Vec<String>,
        actor_type: &str,
        ttl_minutes: i64,
        session_id: Option<Uuid>,
        role_snapshot_id: Option<String>,
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
            session_id: session_id.map(|s| s.to_string()),
            role_snapshot_id,
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

        // Try current key first.
        match jsonwebtoken::decode::<AccessClaims>(token, &self.decoding, &validation) {
            Ok(data) => Ok(data.claims),
            Err(primary_err) => {
                // During rotation overlap: fall back to previous key if present.
                if let Some(ref prev) = self.prev_decoding {
                    if let Ok(data) = jsonwebtoken::decode::<AccessClaims>(token, prev, &validation)
                    {
                        return Ok(data.claims);
                    }
                }
                Err(primary_err.to_string())
            }
        }
    }

    // ---------- JWKS ----------
    pub fn to_jwks(&self) -> Result<Jwks, String> {
        let mut keys = vec![Self::pem_to_jwk_key(&self.public_key_pem, &self.kid)?];

        // Include the previous key during rotation overlap so that remote
        // verifiers (services reading JWKS) can validate both key IDs.
        if let (Some(ref pem), Some(ref kid)) = (&self.prev_public_key_pem, &self.prev_kid) {
            keys.push(Self::pem_to_jwk_key(pem, kid)?);
        }

        Ok(Jwks { keys })
    }

    fn pem_to_jwk_key(pem: &str, kid: &str) -> Result<JwkKey, String> {
        let pubkey = RsaPublicKey::from_public_key_pem(pem).map_err(|e| e.to_string())?;
        let n = URL_SAFE_NO_PAD.encode(pubkey.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(pubkey.e().to_bytes_be());
        Ok(JwkKey {
            kty: "RSA",
            use_: "sig",
            kid: kid.to_string(),
            alg: "RS256",
            n,
            e,
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
            .sign_access_token(
                tenant,
                user,
                roles.clone(),
                perms.clone(),
                actor_type::USER,
                15,
            )
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
        // Basic sign_access_token doesn't set enriched fields
        assert!(decoded.session_id.is_none());
        assert!(decoded.role_snapshot_id.is_none());
    }

    #[test]
    fn enriched_claims_include_session_and_snapshot() {
        let keys = test_keys();
        let tenant = Uuid::new_v4();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let roles = vec!["operator".to_string(), "admin".to_string()];
        let perms = vec![];
        let snapshot = compute_role_snapshot_id(&roles);

        let token = keys
            .sign_access_token_enriched(
                tenant,
                user,
                roles.clone(),
                perms,
                actor_type::USER,
                15,
                Some(session),
                Some(snapshot.clone()),
            )
            .unwrap();

        let decoded = keys.validate_access_token(&token).unwrap();
        assert_eq!(decoded.session_id.as_deref(), Some(session.to_string().as_str()));
        assert_eq!(decoded.role_snapshot_id.as_deref(), Some(snapshot.as_str()));
    }

    #[test]
    fn role_snapshot_is_deterministic_and_order_independent() {
        let a = compute_role_snapshot_id(&["admin".into(), "operator".into()]);
        let b = compute_role_snapshot_id(&["operator".into(), "admin".into()]);
        assert_eq!(a, b, "snapshot must be order-independent");
        assert_eq!(a.len(), 16, "snapshot must be 16-char hex");

        let c = compute_role_snapshot_id(&["admin".into()]);
        assert_ne!(a, c, "different role sets must produce different snapshots");
    }

    #[test]
    fn claims_version_is_set() {
        assert_eq!(CLAIMS_VERSION, "2");
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
            session_id: None,
            role_snapshot_id: None,
            ver: "2".to_string(),
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".to_string());
        let expired_token = jsonwebtoken::encode(&header, &claims, &keys.encoding).unwrap();

        let result = keys.validate_access_token(&expired_token);
        assert!(result.is_err());
        // Drop the zero-TTL token test — it may pass due to timing
        let _ = token;
    }

    /// Zero-downtime rotation: a token issued by the OLD key must still be
    /// accepted by the new verifier during the overlap window when the old
    /// public key is registered via `with_prev_key`.
    #[test]
    fn rotation_overlap_accepts_token_signed_by_prev_key() {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::RsaPrivateKey;

        let mut rng = rand::thread_rng();

        // Generate OLD key pair (retiring)
        let old_priv = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let old_pub = old_priv.to_public_key();
        let old_priv_pem = old_priv.to_pkcs8_pem(LineEnding::LF).unwrap();
        let old_pub_pem = old_pub.to_public_key_pem(LineEnding::LF).unwrap();

        // Generate NEW key pair (current)
        let new_priv = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let new_pub = new_priv.to_public_key();
        let new_priv_pem = new_priv.to_pkcs8_pem(LineEnding::LF).unwrap();
        let new_pub_pem = new_pub.to_public_key_pem(LineEnding::LF).unwrap();

        // Old signer issues a token before rotation completes
        let old_keys = JwtKeys::from_pem(&old_priv_pem, &old_pub_pem, "old-kid".into()).unwrap();
        let token_from_old_key = old_keys
            .sign_access_token(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                actor_type::USER,
                15,
            )
            .unwrap();

        // New signer registers the old public key as prev during overlap
        let mut new_keys =
            JwtKeys::from_pem(&new_priv_pem, &new_pub_pem, "new-kid".into()).unwrap();
        new_keys
            .with_prev_key(&old_pub_pem, "old-kid".into())
            .unwrap();

        // Token issued by old key must be accepted during overlap
        let claims = new_keys.validate_access_token(&token_from_old_key).unwrap();
        assert_eq!(claims.actor_type, "user");

        // Token issued by new key must also be accepted
        let token_from_new_key = new_keys
            .sign_access_token(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                actor_type::USER,
                15,
            )
            .unwrap();
        new_keys.validate_access_token(&token_from_new_key).unwrap();
    }

    /// After overlap ends (prev key removed), tokens signed by the old key
    /// must be rejected.
    #[test]
    fn rotation_overlap_ends_old_token_rejected() {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::RsaPrivateKey;

        let mut rng = rand::thread_rng();

        let old_priv = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let old_pub = old_priv.to_public_key();
        let old_priv_pem = old_priv.to_pkcs8_pem(LineEnding::LF).unwrap();
        let old_pub_pem = old_pub.to_public_key_pem(LineEnding::LF).unwrap();

        let new_priv = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let new_pub = new_priv.to_public_key();
        let new_priv_pem = new_priv.to_pkcs8_pem(LineEnding::LF).unwrap();
        let new_pub_pem = new_pub.to_public_key_pem(LineEnding::LF).unwrap();

        let old_keys = JwtKeys::from_pem(&old_priv_pem, &old_pub_pem, "old-kid".into()).unwrap();
        let token_from_old_key = old_keys
            .sign_access_token(
                Uuid::new_v4(),
                Uuid::new_v4(),
                vec![],
                vec![],
                actor_type::USER,
                15,
            )
            .unwrap();

        // New keys WITHOUT prev key (overlap ended)
        let new_keys = JwtKeys::from_pem(&new_priv_pem, &new_pub_pem, "new-kid".into()).unwrap();

        // Old token must now be rejected
        assert!(new_keys.validate_access_token(&token_from_old_key).is_err());
    }

    /// JWKS endpoint includes both keys during overlap window.
    #[test]
    fn jwks_includes_both_keys_during_overlap() {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::RsaPrivateKey;

        let mut rng = rand::thread_rng();
        let old_priv = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let old_pub_pem = old_priv
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();

        let new_priv = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let new_pub_pem = new_priv
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        let new_priv_pem = new_priv.to_pkcs8_pem(LineEnding::LF).unwrap();

        let mut keys = JwtKeys::from_pem(&new_priv_pem, &new_pub_pem, "new-kid".into()).unwrap();
        keys.with_prev_key(&old_pub_pem, "old-kid".into()).unwrap();

        let jwks = keys.to_jwks().unwrap();
        assert_eq!(
            jwks.keys.len(),
            2,
            "JWKS must include both keys during overlap"
        );

        let kids: Vec<&str> = jwks.keys.iter().map(|k| k.kid.as_str()).collect();
        assert!(kids.contains(&"new-kid"));
        assert!(kids.contains(&"old-kid"));
    }
}

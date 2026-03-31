use axum::http::header::AUTHORIZATION;
use axum::{extract::FromRequestParts, http::request::Parts};
use base64::Engine as _;
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use platform_contracts::portal_identity::{
    PortalAccessClaims, PORTAL_ACTOR_TYPE, PORTAL_AUDIENCE, PORTAL_CLAIMS_VERSION, PORTAL_ISSUER,
};
use platform_http_contracts::ApiError;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone)]
pub struct PortalJwt {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
}

impl PortalJwt {
    pub fn new(private_pem: &str, public_pem: &str) -> Result<Self, String> {
        let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .map_err(|e| format!("invalid portal private key: {e}"))?;
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|e| format!("invalid portal public key: {e}"))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[PORTAL_ISSUER]);
        validation.set_audience(&[PORTAL_AUDIENCE]);
        validation.validate_exp = true;

        Ok(Self {
            encoding,
            decoding,
            validation,
        })
    }

    pub fn issue_access_token(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
        party_id: Uuid,
        scopes: Vec<String>,
        ttl_minutes: i64,
    ) -> Result<String, String> {
        let now = Utc::now();
        let claims = PortalAccessClaims {
            sub: user_id.to_string(),
            iss: PORTAL_ISSUER.to_string(),
            aud: PORTAL_AUDIENCE.to_string(),
            iat: now.timestamp(),
            exp: (now + Duration::minutes(ttl_minutes)).timestamp(),
            jti: Uuid::new_v4().to_string(),
            tenant_id: tenant_id.to_string(),
            party_id: party_id.to_string(),
            actor_type: PORTAL_ACTOR_TYPE.to_string(),
            scopes,
            ver: PORTAL_CLAIMS_VERSION.to_string(),
        };

        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.encoding)
            .map_err(|e| format!("failed to sign portal JWT: {e}"))
    }

    pub fn verify(&self, token: &str) -> Result<PortalAccessClaims, String> {
        jsonwebtoken::decode::<PortalAccessClaims>(token, &self.decoding, &self.validation)
            .map(|data| data.claims)
            .map_err(|_| "unauthorized".to_string())
    }
}

pub fn generate_refresh_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_refresh_token(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

pub struct PortalClaims(pub PortalAccessClaims);

impl<S> FromRequestParts<S> for PortalClaims
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| unauthorized())?;

        let token = auth
            .strip_prefix("Bearer ")
            .ok_or_else(|| unauthorized())?;
        let portal_jwt = parts
            .extensions
            .get::<std::sync::Arc<PortalJwt>>()
            .cloned()
            .ok_or_else(|| unauthorized())?;

        let claims = portal_jwt.verify(token).map_err(|_| unauthorized())?;
        Ok(Self(claims))
    }
}

pub fn unauthorized() -> ApiError {
    ApiError::unauthorized("unauthorized")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_rng_produces_unique_refresh_tokens() {
        let tokens: Vec<String> = (0..100).map(|_| generate_refresh_token()).collect();
        let unique: std::collections::HashSet<&String> = tokens.iter().collect();
        assert_eq!(
            tokens.len(),
            unique.len(),
            "refresh tokens must be unique — OsRng should never repeat"
        );
        assert_eq!(
            tokens[0].len(),
            43,
            "base64url(32 bytes) should be 43 chars"
        );
    }
}

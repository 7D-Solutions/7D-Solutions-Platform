use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessClaims {
    pub sub: String,         // user_id
    pub tenant_id: String,   // tenant_id
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Clone)]
pub struct JwtKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    pub kid: String,
}

impl JwtKeys {
    pub fn from_pem(private_pem: &str, public_pem: &str, kid: String) -> Result<Self, String> {
        let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes()).map_err(|e| e.to_string())?;
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes()).map_err(|e| e.to_string())?;
        Ok(Self { encoding, decoding, kid })
    }

    pub fn sign_access_token(
        &self,
        tenant_id: Uuid,
        user_id: Uuid,
        ttl_minutes: i64,
    ) -> Result<String, String> {
        let now = Utc::now();
        let exp = now + Duration::minutes(ttl_minutes);
        let claims = AccessClaims {
            sub: user_id.to_string(),
            tenant_id: tenant_id.to_string(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
            jti: Uuid::new_v4().to_string(),
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());

        jsonwebtoken::encode(&header, &claims, &self.encoding).map_err(|e| e.to_string())
    }

    pub fn validate_access_token(&self, token: &str) -> Result<AccessClaims, String> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;
        let data = jsonwebtoken::decode::<AccessClaims>(token, &self.decoding, &validation)
            .map_err(|e| e.to_string())?;
        Ok(data.claims)
    }
}

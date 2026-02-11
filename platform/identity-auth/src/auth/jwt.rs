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

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessClaims {
    pub sub: String,        // user_id
    pub tenant_id: String,  // tenant_id
    pub iss: String,        // issuer - prevents cross-environment token reuse
    pub aud: String,        // audience - prevents cross-service token misuse
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
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
        ttl_minutes: i64,
    ) -> Result<String, String> {
        let now = Utc::now();
        let exp = now + Duration::minutes(ttl_minutes);

        let claims = AccessClaims {
            sub: user_id.to_string(),
            tenant_id: tenant_id.to_string(),
            iss: "auth-rs".to_string(),        // issuer
            aud: "7d-platform".to_string(),    // audience
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

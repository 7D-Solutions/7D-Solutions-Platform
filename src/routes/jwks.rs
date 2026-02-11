use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPublicKey;
use serde::Serialize;
use std::sync::Arc;

use crate::auth::jwt::JwtKeys;

#[derive(Clone)]
pub struct JwksState {
    pub jwt: JwtKeys,
}

#[derive(Serialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Serialize)]
struct Jwk {
    kty: &'static str,
    kid: String,
    #[serde(rename = "use")]
    use_: &'static str,
    alg: &'static str,
    n: String,
    e: String,
}

pub async fn jwks(State(state): State<Arc<JwksState>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let public_pem = state.jwt.public_key_pem();

    let pubkey = RsaPublicKey::from_public_key_pem(&public_pem)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("jwks parse error: {e}")))?;

    let n_bytes = pubkey.n().to_bytes_be();
    let e_bytes = pubkey.e().to_bytes_be();

    let n = URL_SAFE_NO_PAD.encode(n_bytes);
    let e = URL_SAFE_NO_PAD.encode(e_bytes);

    Ok(Json(Jwks {
        keys: vec![Jwk {
            kty: "RSA",
            kid: state.jwt.kid().to_string(),
            use_: "sig",
            alg: "RS256",
            n,
            e,
        }],
    }))
}

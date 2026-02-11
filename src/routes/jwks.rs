use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

use crate::auth::jwt::{Jwks, JwtKeys};

#[derive(Clone)]
pub struct JwksState {
    pub jwt: JwtKeys,
}

pub async fn jwks_handler(
    State(state): State<Arc<JwksState>>,
) -> Result<Json<Jwks>, (StatusCode, String)> {
    state
        .jwt
        .to_jwks()
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

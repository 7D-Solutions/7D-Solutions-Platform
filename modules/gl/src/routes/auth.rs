//! Shared auth helper for GL route handlers.

use axum::{http::StatusCode, Extension};
use security::VerifiedClaims;

/// Extract tenant_id from JWT claims.
/// Returns UNAUTHORIZED if claims are missing.
pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, String)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            "Missing or invalid authentication".to_string(),
        )),
    }
}

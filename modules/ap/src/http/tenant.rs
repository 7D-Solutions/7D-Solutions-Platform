//! Shared tenant extraction from JWT claims for AP handlers.

use axum::http::StatusCode;
use axum::{Extension, Json};
use security::VerifiedClaims;

use super::admin_types::ErrorBody;

/// Extract the tenant ID string from verified JWT claims in request extensions.
///
/// Returns `Err(401)` if no claims are present (unauthenticated request).
/// All AP route handlers should use this instead of header-based tenant extraction.
pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
    }
}

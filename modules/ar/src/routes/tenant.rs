use axum::http::StatusCode;
use axum::Json;
use security::VerifiedClaims;

use crate::models::ErrorResponse;

/// Extract the tenant ID string from verified JWT claims in request extensions.
///
/// Returns `Err(401)` if no claims are present (unauthenticated request).
/// All AR route handlers should use this instead of hardcoded tenant strings.
pub fn extract_tenant(
    claims: &Option<axum::Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    match claims {
        Some(axum::Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
    }
}

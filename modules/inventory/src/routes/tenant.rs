use axum::{http::StatusCode, Extension, Json};
use security::VerifiedClaims;
use serde_json::{json, Value};

/// Extract the tenant ID string from verified JWT claims in request extensions.
///
/// Returns `Err(401)` if no claims are present (unauthenticated request).
pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<Value>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized",
                "message": "Missing or invalid authentication"
            })),
        )),
    }
}

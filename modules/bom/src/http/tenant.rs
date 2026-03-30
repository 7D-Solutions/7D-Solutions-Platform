use axum::Extension;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;

pub fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError::unauthorized(
            "Missing or invalid authentication",
        )),
    }
}

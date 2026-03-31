use axum::Extension;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;

/// Extract tenant ID from JWT claims.
pub fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Ok("dev-tenant".to_string()),
    }
}

/// Enrich an `ApiError` with the `request_id` from `TracingContext`.
pub fn with_request_id(err: ApiError, ctx: &Option<Extension<TracingContext>>) -> ApiError {
    match ctx {
        Some(Extension(c)) => {
            if let Some(tid) = &c.trace_id {
                err.with_request_id(tid.clone())
            } else {
                err
            }
        }
        None => err,
    }
}

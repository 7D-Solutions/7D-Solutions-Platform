use axum::Extension;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;

/// Extract the internal actor from JWT claims (for admin routes).
pub fn extract_actor(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<&VerifiedClaims, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c),
        None => Err(ApiError::unauthorized("Missing or invalid authentication")),
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

//! Tenant extraction and request-ID enrichment helpers.

use axum::Extension;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;

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

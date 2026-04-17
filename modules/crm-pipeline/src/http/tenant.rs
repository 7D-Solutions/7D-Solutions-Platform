//! Shared HTTP utilities for CRM Pipeline handlers.

use axum::Extension;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;

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

pub fn correlation_from_headers(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

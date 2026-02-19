//! HTTP header extraction for distributed tracing context.
//!
//! Extracts `X-Trace-Id`, `X-Correlation-Id`, `X-Actor-Id`, and `X-Actor-Type`
//! from incoming HTTP requests and packages them into a [`TracingContext`]
//! that can be applied to outgoing [`EventEnvelope`]s.
//!
//! # Standard Headers
//!
//! | Header             | Maps to                     | Behavior if missing         |
//! |--------------------|-----------------------------|-----------------------------|
//! | `X-Trace-Id`       | `TracingContext.trace_id`    | Auto-generated UUID         |
//! | `X-Correlation-Id` | `TracingContext.correlation_id` | Falls back to trace_id   |
//! | `X-Actor-Id`       | `TracingContext.actor_id`    | None (anonymous)            |
//! | `X-Actor-Type`     | `TracingContext.actor_type`  | None                        |

use axum::{extract::Request, middleware::Next, response::Response};
use event_bus::TracingContext;
use http::HeaderMap;
use uuid::Uuid;

/// Extract a [`TracingContext`] from HTTP request headers.
///
/// If `X-Trace-Id` is absent, a new UUID is generated (ensuring every
/// request has a trace). If `X-Correlation-Id` is absent, it inherits
/// the trace_id value (reasonable default for single-hop requests).
pub fn tracing_context_from_headers(headers: &HeaderMap) -> TracingContext {
    let trace_id = header_string(headers, "x-trace-id")
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let correlation_id = header_string(headers, "x-correlation-id")
        .unwrap_or_else(|| trace_id.clone());

    let actor_id = header_string(headers, "x-actor-id")
        .and_then(|s| Uuid::parse_str(&s).ok());

    let actor_type = header_string(headers, "x-actor-type");

    let mut ctx = TracingContext::new()
        .with_trace_id(trace_id)
        .with_correlation_id(correlation_id);

    if let (Some(id), Some(at)) = (actor_id, actor_type) {
        ctx = ctx.with_actor(id, at);
    }

    ctx
}

/// Axum middleware that injects [`TracingContext`] into request extensions.
///
/// Add this to your router's middleware stack:
/// ```ignore
/// use axum::Router;
/// use security::tracing::tracing_context_middleware;
///
/// let app = Router::new()
///     // ... routes ...
///     .layer(axum::middleware::from_fn(tracing_context_middleware));
/// ```
///
/// Handlers can then extract the context:
/// ```rust,no_run
/// use axum::Extension;
/// use event_bus::TracingContext;
///
/// async fn my_handler(Extension(ctx): Extension<TracingContext>) {
///     // ctx.trace_id, ctx.correlation_id, ctx.actor_id, etc.
/// }
/// ```
pub async fn tracing_context_middleware(request: Request, next: Next) -> Response {
    let ctx = tracing_context_from_headers(request.headers());

    let mut request = request;
    request.extensions_mut().insert(ctx);
    next.run(request).await
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;

    #[test]
    fn test_auto_generates_trace_id_when_missing() {
        let headers = HeaderMap::new();
        let ctx = tracing_context_from_headers(&headers);

        assert!(ctx.trace_id.is_some());
        // Should be a valid UUID
        let tid = ctx.trace_id.unwrap();
        assert!(Uuid::parse_str(&tid).is_ok());
    }

    #[test]
    fn test_correlation_id_falls_back_to_trace_id() {
        let headers = HeaderMap::new();
        let ctx = tracing_context_from_headers(&headers);

        assert_eq!(ctx.trace_id, ctx.correlation_id);
    }

    #[test]
    fn test_extracts_all_headers() {
        let actor_id = Uuid::new_v4();
        let mut headers = HeaderMap::new();
        headers.insert("x-trace-id", "trace-123".parse().unwrap());
        headers.insert("x-correlation-id", "corr-456".parse().unwrap());
        headers.insert("x-actor-id", actor_id.to_string().parse().unwrap());
        headers.insert("x-actor-type", "User".parse().unwrap());

        let ctx = tracing_context_from_headers(&headers);

        assert_eq!(ctx.trace_id.as_deref(), Some("trace-123"));
        assert_eq!(ctx.correlation_id.as_deref(), Some("corr-456"));
        assert_eq!(ctx.actor_id, Some(actor_id));
        assert_eq!(ctx.actor_type.as_deref(), Some("User"));
    }

    #[test]
    fn test_ignores_empty_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-trace-id", "".parse().unwrap());
        headers.insert("x-actor-id", "".parse().unwrap());

        let ctx = tracing_context_from_headers(&headers);

        // Empty trace-id → auto-generated
        assert!(ctx.trace_id.is_some());
        let tid = ctx.trace_id.unwrap();
        assert!(Uuid::parse_str(&tid).is_ok());
        // Empty actor-id → None
        assert!(ctx.actor_id.is_none());
    }

    #[test]
    fn test_actor_requires_both_id_and_type() {
        let actor_id = Uuid::new_v4();
        let mut headers = HeaderMap::new();
        headers.insert("x-actor-id", actor_id.to_string().parse().unwrap());
        // No x-actor-type

        let ctx = tracing_context_from_headers(&headers);

        // Actor not set because type is missing
        assert!(ctx.actor_id.is_none());
        assert!(ctx.actor_type.is_none());
    }

    #[test]
    fn test_roundtrip_headers_to_envelope() {
        let actor_id = Uuid::new_v4();
        let mut headers = HeaderMap::new();
        headers.insert("x-trace-id", "trace-rt".parse().unwrap());
        headers.insert("x-correlation-id", "corr-rt".parse().unwrap());
        headers.insert("x-actor-id", actor_id.to_string().parse().unwrap());
        headers.insert("x-actor-type", "Service".parse().unwrap());

        let ctx = tracing_context_from_headers(&headers);

        let envelope = event_bus::EventEnvelope::new(
            "tenant-1".to_string(),
            "test".to_string(),
            "test.event".to_string(),
            serde_json::json!({}),
        )
        .with_tracing_context(&ctx);

        assert_eq!(envelope.trace_id.as_deref(), Some("trace-rt"));
        assert_eq!(envelope.correlation_id.as_deref(), Some("corr-rt"));
        assert_eq!(envelope.actor_id, Some(actor_id));
        assert_eq!(envelope.actor_type.as_deref(), Some("Service"));
    }
}

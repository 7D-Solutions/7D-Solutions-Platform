//! Rate limiting middleware for AR service
//!
//! This middleware enforces tenant-aware rate limits on API endpoints.
//! Different limits apply to normal reads vs. fallback paths.

use axum::{
    extract::Request,
    http::{StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use security::ratelimit::RateLimiter;
use std::sync::Arc;
use tracing::{debug, warn};

/// Rate limiting middleware state
pub struct RateLimitState {
    pub limiter: Arc<RateLimiter>,
}

/// Rate limiting middleware for read endpoints
///
/// Enforces tenant-aware rate limits. Extracts tenant_id from request path
/// and checks against configured quotas.
///
/// # Example
///
/// ```ignore
/// use axum::{middleware, Router};
/// use security::ratelimit::RateLimiter;
/// use std::sync::Arc;
///
/// let limiter = Arc::new(RateLimiter::new());
/// let state = RateLimitState { limiter };
///
/// let app = Router::new()
///     .route("/api/tenants/:tenant_id/invoices", get(list_invoices))
///     .layer(middleware::from_fn_with_state(
///         Arc::new(state),
///         ratelimit_middleware
///     ));
/// ```
pub async fn ratelimit_middleware(
    axum::extract::State(state): axum::extract::State<Arc<RateLimitState>>,
    uri: Uri,
    request: Request,
    next: Next,
) -> Response {
    // Extract tenant_id from path
    // Assumes path format: /api/tenants/:tenant_id/...
    let path = uri.path();
    let tenant_id = extract_tenant_id_from_path(path);

    if let Some(tenant_id) = tenant_id {
        // Determine if this is a fallback path
        let is_fallback = path.contains("/fallback/") || path.contains("/_fallback_");

        // Check rate limit
        let result = if is_fallback {
            state.limiter.check_fallback_limit(&tenant_id, path)
        } else {
            state.limiter.check_limit(&tenant_id, path)
        };

        match result {
            Ok(()) => {
                debug!(
                    tenant_id = %tenant_id,
                    path = %path,
                    is_fallback = is_fallback,
                    "Rate limit check passed"
                );
                next.run(request).await
            }
            Err(err) => {
                warn!(
                    tenant_id = %tenant_id,
                    path = %path,
                    is_fallback = is_fallback,
                    error = %err,
                    "Rate limit exceeded"
                );

                (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "error": "rate_limit_exceeded",
                        "message": format!("Rate limit exceeded for tenant {}", tenant_id),
                        "retry_after": 60
                    })),
                )
                    .into_response()
            }
        }
    } else {
        // No tenant_id in path, allow request to proceed
        // (might be a health check or other non-tenant endpoint)
        next.run(request).await
    }
}

/// Extract tenant_id from request path
///
/// Supports path patterns:
/// - /api/tenants/:tenant_id/...
/// - /api/:tenant_id/...
///
/// Returns None if no tenant_id found in path.
fn extract_tenant_id_from_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();

    // Look for /tenants/:tenant_id/ pattern
    if let Some(idx) = parts.iter().position(|&p| p == "tenants") {
        if idx + 1 < parts.len() {
            let tenant_id = parts[idx + 1];
            if !tenant_id.is_empty() && tenant_id != "tenants" {
                return Some(tenant_id.to_string());
            }
        }
    }

    // Look for /api/:tenant_id/ pattern (alternative)
    if parts.len() >= 3 && parts[1] == "api" {
        let tenant_id = parts[2];
        if !tenant_id.is_empty()
            && tenant_id != "version"
            && tenant_id != "health"
            && tenant_id != "ready"
            && tenant_id != "metrics"
        {
            return Some(tenant_id.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tenant_id_from_path() {
        assert_eq!(
            extract_tenant_id_from_path("/api/tenants/t123/invoices"),
            Some("t123".to_string())
        );

        assert_eq!(
            extract_tenant_id_from_path("/api/tenants/tenant-456/payments"),
            Some("tenant-456".to_string())
        );

        assert_eq!(
            extract_tenant_id_from_path("/api/t789/invoices"),
            Some("t789".to_string())
        );

        assert_eq!(extract_tenant_id_from_path("/api/health"), None);

        assert_eq!(extract_tenant_id_from_path("/api/version"), None);

        assert_eq!(extract_tenant_id_from_path("/metrics"), None);
    }

    #[test]
    fn test_fallback_path_detection() {
        let path1 = "/api/tenants/t123/fallback/invoices";
        assert!(path1.contains("/fallback/"));

        let path2 = "/api/tenants/t123/_fallback_invoices";
        assert!(path2.contains("/_fallback_"));

        let path3 = "/api/tenants/t123/invoices";
        assert!(!path3.contains("/fallback/"));
        assert!(!path3.contains("/_fallback_"));
    }
}

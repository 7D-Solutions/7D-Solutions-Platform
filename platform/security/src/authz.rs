//! Authorization enforcement middleware (RBAC seam)
//!
//! Provides a Tower Layer that all services mount. In permissive mode (default),
//! requests pass through with an `AuthzStatus` extension attached. In strict mode
//! (deny-by-default), unauthenticated requests receive 401 Unauthorized.
//!
//! Phase 35 will fill in actual token validation and role resolution. This module
//! only provides the enforcement hook and the strict-mode toggle.

use axum::{
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::warn;

/// Configuration controlling authorization enforcement behaviour.
#[derive(Debug, Clone)]
pub struct AuthzConfig {
    /// When true, unauthenticated requests are rejected (deny-by-default).
    /// When false (default), requests pass through with `AuthzStatus::Unauthenticated`.
    pub strict: bool,
}

impl AuthzConfig {
    /// Read configuration from the `AUTHZ_STRICT` environment variable.
    /// Defaults to permissive mode when the variable is absent.
    pub fn from_env() -> Self {
        Self {
            strict: std::env::var("AUTHZ_STRICT")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
        }
    }

    pub fn permissive() -> Self {
        Self { strict: false }
    }

    pub fn strict() -> Self {
        Self { strict: true }
    }
}

impl Default for AuthzConfig {
    fn default() -> Self {
        Self::permissive()
    }
}

/// Marker inserted into request extensions by the authz middleware.
/// Downstream handlers can extract this to inspect authentication state.
#[derive(Debug, Clone)]
pub enum AuthzStatus {
    /// Request carried valid credentials (populated by Phase 35 token validation).
    Authenticated {
        user_id: String,
        tenant_id: String,
    },
    /// No valid credentials found.
    Unauthenticated,
}

/// Tower Layer that wraps services with authorization enforcement.
///
/// Usage in service `main.rs`:
/// ```ignore
/// use security::authz::AuthzLayer;
///
/// let app = Router::new()
///     .route("/api/health", get(health))
///     .with_state(app_state)
///     .layer(AuthzLayer::from_env())
///     .layer(cors);
/// ```
#[derive(Debug, Clone)]
pub struct AuthzLayer {
    config: AuthzConfig,
}

impl AuthzLayer {
    pub fn new(config: AuthzConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Self {
        Self::new(AuthzConfig::from_env())
    }
}

impl<S> Layer<S> for AuthzLayer {
    type Service = AuthzMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthzMiddleware {
            inner,
            config: self.config.clone(),
        }
    }
}

/// Middleware service that enforces authorization policy.
#[derive(Debug, Clone)]
pub struct AuthzMiddleware<S> {
    inner: S,
    config: AuthzConfig,
}

impl<S> Service<Request> for AuthzMiddleware<S>
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>> {
        let config = self.config.clone();
        // Standard Tower clone-swap: take the ready service, leave a fresh clone.
        let cloned = self.inner.clone();
        let mut ready_svc = std::mem::replace(&mut self.inner, cloned);

        Box::pin(async move {
            let _auth_header = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok());

            // Phase 35 will add token validation here and resolve to Authenticated.
            // Until then, every request is marked Unauthenticated.
            let status = AuthzStatus::Unauthenticated;

            if config.strict && matches!(status, AuthzStatus::Unauthenticated) {
                warn!(
                    path = %req.uri().path(),
                    "authz: strict mode rejected unauthenticated request"
                );
                return Ok(StatusCode::UNAUTHORIZED.into_response());
            }

            req.extensions_mut().insert(status);
            ready_svc.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_permissive() {
        let cfg = AuthzConfig::default();
        assert!(!cfg.strict);
    }

    #[test]
    fn config_strict_constructor() {
        let cfg = AuthzConfig::strict();
        assert!(cfg.strict);
    }

    #[test]
    fn config_permissive_constructor() {
        let cfg = AuthzConfig::permissive();
        assert!(!cfg.strict);
    }

    #[test]
    fn layer_clones_config() {
        let layer = AuthzLayer::new(AuthzConfig::strict());
        let layer2 = layer.clone();
        assert!(layer2.config.strict);
    }
}

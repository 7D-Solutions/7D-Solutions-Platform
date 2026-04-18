use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::response::Response;
use tower::{Layer, Service};

use crate::config::{read_cookie, WebAuthConfig};

/// Tower Layer that reads the access cookie and attaches [`security::VerifiedClaims`]
/// to request extensions when the cookie is present and the JWT is valid.
///
/// **Permissive**: passes through silently when no cookie is present or when the
/// JWT fails verification. Downstream auth guards (RequirePermissionsLayer, etc.)
/// are responsible for returning 401 when claims are missing.
///
/// Apply globally so all routes benefit:
/// ```rust,ignore
/// app.nest("/api/auth", auth_router).layer(cookie_mw);
/// ```
#[derive(Clone)]
pub struct CookieAuthLayer {
    config: Arc<WebAuthConfig>,
}

impl CookieAuthLayer {
    pub fn new(config: Arc<WebAuthConfig>) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for CookieAuthLayer {
    type Service = CookieAuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CookieAuthMiddleware {
            inner,
            config: self.config.clone(),
        }
    }
}

/// Middleware service produced by [`CookieAuthLayer`].
#[derive(Clone)]
pub struct CookieAuthMiddleware<S> {
    inner: S,
    config: Arc<WebAuthConfig>,
}

impl<S> Service<Request> for CookieAuthMiddleware<S>
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

    fn call(&mut self, mut req: Request) -> Self::Future {
        let config = self.config.clone();
        let cloned = self.inner.clone();
        let mut ready_svc = std::mem::replace(&mut self.inner, cloned);

        Box::pin(async move {
            if let Some(verifier) = config.jwt_verifier.as_ref() {
                if let Some(token) = read_cookie(req.headers(), &config.access_cookie_name()) {
                    match verifier.verify(&token) {
                        Ok(claims) => {
                            req.extensions_mut().insert(claims);
                        }
                        Err(e) => {
                            // Passes through — downstream route decides if 401 is warranted.
                            tracing::debug!(error = %e, "access cookie JWT invalid — no claims inserted");
                        }
                    }
                }
            }
            ready_svc.call(req).await
        })
    }
}

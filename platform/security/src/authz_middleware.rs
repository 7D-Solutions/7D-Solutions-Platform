//! Authz middleware: extract JWT claims and enforce permissions.
//!
//! Two composable Tower layers for Axum routers:
//!
//! - [`ClaimsLayer`]: Extracts and verifies the Bearer token on every request,
//!   inserting [`VerifiedClaims`](crate::claims::VerifiedClaims) into extensions.
//! - [`RequirePermissionsLayer`]: Per-route guard that checks the caller holds
//!   all required permission strings. Returns 403 when permissions are missing.
//!
//! # Usage
//!
//! ```ignore
//! use security::authz_middleware::{ClaimsLayer, RequirePermissionsLayer};
//! use security::claims::JwtVerifier;
//! use std::sync::Arc;
//!
//! let verifier = Arc::new(JwtVerifier::from_public_pem(&pem).unwrap());
//!
//! let app = Router::new()
//!     .route("/invoices", post(create_invoice))
//!     .route_layer(RequirePermissionsLayer::new(&["ar.create"]))
//!     .route("/health", get(health))
//!     .layer(ClaimsLayer::permissive(verifier));
//! ```

use crate::audit_log::{security_event, SecurityOutcome};
use crate::claims::{JwtVerifier, VerifiedClaims};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

// ── Claims Extraction ──────────────────────────────────────────────

/// Tower Layer that verifies the JWT Bearer token and attaches
/// [`VerifiedClaims`] to request extensions.
///
/// In **strict** mode, requests without a valid token receive 401.
/// In **permissive** mode (default), requests pass through without claims.
#[derive(Clone)]
pub struct ClaimsLayer {
    verifier: Arc<JwtVerifier>,
    strict: bool,
}

impl ClaimsLayer {
    pub fn new(verifier: Arc<JwtVerifier>, strict: bool) -> Self {
        Self { verifier, strict }
    }

    /// Permissive mode — invalid/missing tokens pass through without claims.
    pub fn permissive(verifier: Arc<JwtVerifier>) -> Self {
        Self::new(verifier, false)
    }

    /// Strict mode — invalid/missing tokens receive 401 Unauthorized.
    pub fn strict(verifier: Arc<JwtVerifier>) -> Self {
        Self::new(verifier, true)
    }
}

impl<S> Layer<S> for ClaimsLayer {
    type Service = ClaimsMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ClaimsMiddleware {
            inner,
            verifier: self.verifier.clone(),
            strict: self.strict,
        }
    }
}

/// Middleware service created by [`ClaimsLayer`].
#[derive(Clone)]
pub struct ClaimsMiddleware<S> {
    inner: S,
    verifier: Arc<JwtVerifier>,
    strict: bool,
}

impl<S> Service<Request> for ClaimsMiddleware<S>
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
        let verifier = self.verifier.clone();
        let strict = self.strict;
        let cloned = self.inner.clone();
        let mut ready_svc = std::mem::replace(&mut self.inner, cloned);

        Box::pin(async move {
            let claims = extract_bearer(&req).map(|token| verifier.verify(token));

            match &claims {
                Some(Ok(verified)) => {
                    req.extensions_mut().insert(verified.clone());
                }
                Some(Err(e)) => {
                    // Token was present but verification failed — always log this
                    // even in permissive mode, because a signed token with bad claims
                    // can indicate a compromised identity service or injection attempt.
                    let path = req.uri().path().to_string();
                    tracing::warn!(
                        path = %path,
                        error = %e,
                        "JWT verification failed — token present but rejected"
                    );
                    if strict {
                        security_event(
                            None,
                            None,
                            &path,
                            SecurityOutcome::Denied,
                            &format!("strict mode rejected — {e}"),
                        );
                        let body = serde_json::json!({
                            "error": "unauthorized",
                            "message": "Missing or invalid authentication"
                        });
                        return Ok((StatusCode::UNAUTHORIZED, axum::Json(body)).into_response());
                    }
                }
                None => {}
            }

            if claims.is_none() && strict {
                let path = req.uri().path().to_string();
                security_event(
                    None,
                    None,
                    &path,
                    SecurityOutcome::Denied,
                    "strict mode rejected — no valid claims",
                );
                let body = serde_json::json!({
                    "error": "unauthorized",
                    "message": "Missing or invalid authentication"
                });
                return Ok((StatusCode::UNAUTHORIZED, axum::Json(body)).into_response());
            }

            ready_svc.call(req).await
        })
    }
}

/// Extract the Bearer token string from the Authorization header.
fn extract_bearer(req: &Request) -> Option<&str> {
    req.headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

// ── Optional Claims Middleware (axum::middleware::from_fn_with_state) ─────────

/// Axum middleware function for optional JWT claims extraction.
///
/// Attach via `axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw)`.
///
/// If the verifier is `None` (e.g. `JWT_PUBLIC_KEY` not set in dev), or if the
/// bearer token is missing / invalid, no claims are inserted and the request
/// continues.  Mutation routes protected by [`RequirePermissionsLayer`] will
/// then return **401 Unauthorized** because no claims are present.
///
/// # Example
///
/// ```ignore
/// use security::{JwtVerifier, optional_claims_mw};
/// use std::sync::Arc;
///
/// let verifier: Option<Arc<JwtVerifier>> = JwtVerifier::from_env().map(Arc::new);
///
/// let app = Router::new()
///     .route("/api/x", post(create))
///     .route_layer(RequirePermissionsLayer::new(&["x.mutate"]))
///     .route("/api/x", get(list))
///     .layer(axum::middleware::from_fn_with_state(verifier, optional_claims_mw));
/// ```
pub async fn optional_claims_mw(
    State(verifier): State<Option<Arc<JwtVerifier>>>,
    mut req: Request,
    next: Next,
) -> Response {
    if let Some(v) = verifier.as_deref() {
        if let Some(claims) = extract_bearer(&req).and_then(|t| v.verify(t).ok()) {
            req.extensions_mut().insert(claims);
        }
    }
    next.run(req).await
}

// ── Permission Enforcement ─────────────────────────────────────────

/// Tower Layer that enforces required permission strings on a route.
///
/// Reads [`VerifiedClaims`] from request extensions (set by [`ClaimsLayer`]).
/// Returns **403 Forbidden** if any required permission is missing, or
/// **401 Unauthorized** if no claims are present at all.
#[derive(Clone)]
pub struct RequirePermissionsLayer {
    required: Arc<[String]>,
}

impl RequirePermissionsLayer {
    pub fn new(perms: &[&str]) -> Self {
        Self {
            required: perms.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

impl<S> Layer<S> for RequirePermissionsLayer {
    type Service = RequirePermissionsMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequirePermissionsMiddleware {
            inner,
            required: self.required.clone(),
        }
    }
}

/// Middleware service created by [`RequirePermissionsLayer`].
#[derive(Clone)]
pub struct RequirePermissionsMiddleware<S> {
    inner: S,
    required: Arc<[String]>,
}

impl<S> Service<Request> for RequirePermissionsMiddleware<S>
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

    fn call(&mut self, req: Request) -> Self::Future {
        let required = self.required.clone();
        let cloned = self.inner.clone();
        let mut ready_svc = std::mem::replace(&mut self.inner, cloned);

        Box::pin(async move {
            match req.extensions().get::<VerifiedClaims>() {
                Some(claims) => {
                    let missing: Vec<&str> = required
                        .iter()
                        .filter(|p| !claims.perms.contains(p))
                        .map(|s| s.as_str())
                        .collect();

                    if !missing.is_empty() {
                        let path = req.uri().path().to_string();
                        security_event(
                            Some(claims.tenant_id),
                            Some(claims.user_id),
                            &path,
                            SecurityOutcome::Denied,
                            &format!("insufficient permissions, missing: {missing:?}"),
                        );
                        let mut body = serde_json::json!({
                            "error": "forbidden",
                            "message": "Insufficient permissions"
                        });
                        if let Some(ctx) = req.extensions().get::<event_bus::TracingContext>() {
                            if let Some(tid) = &ctx.trace_id {
                                body["request_id"] = serde_json::Value::String(tid.clone());
                            }
                        }
                        return Ok((StatusCode::FORBIDDEN, axum::Json(body)).into_response());
                    }
                }
                None => {
                    let path = req.uri().path().to_string();
                    security_event(
                        None,
                        None,
                        &path,
                        SecurityOutcome::Denied,
                        "no claims present — permission check failed",
                    );
                    let mut body = serde_json::json!({
                        "error": "unauthorized",
                        "message": "Missing or invalid authentication"
                    });
                    if let Some(ctx) = req.extensions().get::<event_bus::TracingContext>() {
                        if let Some(tid) = &ctx.trace_id {
                            body["request_id"] = serde_json::Value::String(tid.clone());
                        }
                    }
                    return Ok((StatusCode::UNAUTHORIZED, axum::Json(body)).into_response());
                }
            }

            ready_svc.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use http::Request as HttpRequest;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::RsaPrivateKey;
    use serde::Serialize;
    use tower::ServiceExt;
    use uuid::Uuid;

    // ── Test helpers ───────────────────────────────────────────────

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        iss: String,
        aud: String,
        iat: i64,
        exp: i64,
        jti: String,
        tenant_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        app_id: Option<String>,
        roles: Vec<String>,
        perms: Vec<String>,
        actor_type: String,
        ver: String,
    }

    struct TestKeys {
        encoding: EncodingKey,
        verifier: Arc<JwtVerifier>,
    }

    fn make_test_keys() -> TestKeys {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key generation");
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).expect("PEM encoding");
        let pub_pem = pub_key
            .to_public_key_pem(LineEnding::LF)
            .expect("public PEM");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
        let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).expect("JWT verifier"));
        TestKeys { encoding, verifier }
    }

    fn sign_token(enc: &EncodingKey, claims: &TestClaims) -> String {
        let header = Header::new(Algorithm::RS256);
        jsonwebtoken::encode(&header, claims, enc).expect("sign token")
    }

    fn default_claims(perms: Vec<String>) -> TestClaims {
        let now = chrono::Utc::now();
        TestClaims {
            sub: Uuid::new_v4().to_string(),
            iss: "auth-rs".to_string(),
            aud: "7d-platform".to_string(),
            iat: now.timestamp(),
            exp: (now + chrono::Duration::minutes(15)).timestamp(),
            jti: Uuid::new_v4().to_string(),
            tenant_id: Uuid::new_v4().to_string(),
            app_id: None,
            roles: vec!["operator".into()],
            perms,
            actor_type: "user".to_string(),
            ver: "1".to_string(),
        }
    }

    fn test_router(layer: ClaimsLayer) -> Router {
        Router::new()
            .route("/open", get(|| async { "ok" }))
            .layer(layer)
    }

    fn bearer(token: &str) -> String {
        format!("Bearer {token}")
    }

    // ── ClaimsLayer tests ──────────────────────────────────────────

    #[tokio::test]
    async fn authz_permissive_passes_without_token() {
        let keys = make_test_keys();
        let app = test_router(ClaimsLayer::permissive(keys.verifier));

        let req = HttpRequest::builder()
            .uri("/open")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authz_strict_rejects_without_token() {
        let keys = make_test_keys();
        let app = test_router(ClaimsLayer::strict(keys.verifier));

        let req = HttpRequest::builder()
            .uri("/open")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authz_strict_accepts_valid_token() {
        let keys = make_test_keys();
        let claims = default_claims(vec!["ar.read".into()]);
        let token = sign_token(&keys.encoding, &claims);
        let app = test_router(ClaimsLayer::strict(keys.verifier));

        let req = HttpRequest::builder()
            .uri("/open")
            .header("authorization", bearer(&token))
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authz_strict_rejects_invalid_token() {
        let keys = make_test_keys();
        let app = test_router(ClaimsLayer::strict(keys.verifier));

        let req = HttpRequest::builder()
            .uri("/open")
            .header("authorization", "Bearer not-a-real-jwt")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authz_permissive_ignores_invalid_token() {
        let keys = make_test_keys();
        let app = test_router(ClaimsLayer::permissive(keys.verifier));

        let req = HttpRequest::builder()
            .uri("/open")
            .header("authorization", "Bearer garbage")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── RequirePermissionsLayer tests ──────────────────────────────

    fn guarded_router(keys: &TestKeys) -> Router {
        Router::new()
            .route("/guarded", get(|| async { "ok" }))
            .route_layer(RequirePermissionsLayer::new(&["ar.create", "ar.read"]))
            .layer(ClaimsLayer::permissive(keys.verifier.clone()))
    }

    #[tokio::test]
    async fn authz_require_perms_grants_with_all_perms() {
        let keys = make_test_keys();
        let claims = default_claims(vec!["ar.create".into(), "ar.read".into(), "gl.post".into()]);
        let token = sign_token(&keys.encoding, &claims);
        let app = guarded_router(&keys);

        let req = HttpRequest::builder()
            .uri("/guarded")
            .header("authorization", bearer(&token))
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authz_require_perms_denies_with_missing_perm() {
        let keys = make_test_keys();
        // Only has ar.read, missing ar.create
        let claims = default_claims(vec!["ar.read".into()]);
        let token = sign_token(&keys.encoding, &claims);
        let app = guarded_router(&keys);

        let req = HttpRequest::builder()
            .uri("/guarded")
            .header("authorization", bearer(&token))
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn authz_require_perms_denies_without_claims() {
        let keys = make_test_keys();
        let app = guarded_router(&keys);

        let req = HttpRequest::builder()
            .uri("/guarded")
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authz_require_perms_exact_match() {
        let keys = make_test_keys();
        // Has exactly the required permissions, no more
        let claims = default_claims(vec!["ar.create".into(), "ar.read".into()]);
        let token = sign_token(&keys.encoding, &claims);
        let app = guarded_router(&keys);

        let req = HttpRequest::builder()
            .uri("/guarded")
            .header("authorization", bearer(&token))
            .body(Body::empty())
            .expect("build request");
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

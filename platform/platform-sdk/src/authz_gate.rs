//! Route-level permission enforcement middleware (AuthzGate).
//!
//! Provides opt-in authorization checks per `(Method, path)` pair. Routes not
//! listed in the config are unprotected and pass through freely — this is
//! intentional: the guard is **opt-in**, not deny-by-default.
//!
//! ## Key properties
//! - **Opt-in**: routes absent from the config map pass through without any check.
//! - **Superuser bypass**: `admin:all` in the user's permissions bypasses every check.
//! - **Any-of match**: a route passes if the user holds *at least one* of the
//!   configured permissions for that route.
//! - **Exact path match**: paths are matched literally — no wildcards or templates.
//!   Wildcard support is a follow-up concern.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use platform_sdk::authz_gate::AuthzGateConfig;
//! use axum::http::Method;
//!
//! let config = AuthzGateConfig::new([
//!     ((Method::GET,  "/api/invoices"),       vec!["invoices:read"]),
//!     ((Method::POST, "/api/invoices"),        vec!["invoices:write"]),
//!     ((Method::DELETE, "/api/invoices/{id}"), vec!["invoices:delete", "invoices:admin"]),
//! ]);
//!
//! ModuleBuilder::from_manifest("module.toml")
//!     .authz_gate(config)
//!     .routes(|ctx| { /* ... */ })
//!     .run()
//!     .await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use security::claims::VerifiedClaims;

/// Configuration for the AuthzGate middleware.
///
/// Maps `(Method, path)` pairs to the set of permissions required to access
/// that route. The user must hold **at least one** permission from the vec.
/// Routes absent from the map are unprotected.
#[derive(Debug, Clone, Default)]
pub struct AuthzGateConfig {
    rules: HashMap<(Method, String), Vec<String>>,
}

impl AuthzGateConfig {
    /// Create a new config from an iterator of `((Method, path), permissions)` entries.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use axum::http::Method;
    /// use platform_sdk::authz_gate::AuthzGateConfig;
    ///
    /// let config = AuthzGateConfig::new([
    ///     ((Method::GET, "/api/orders"), vec!["orders:read"]),
    ///     ((Method::POST, "/api/orders"), vec!["orders:write"]),
    /// ]);
    /// ```
    pub fn new<I, P, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = ((Method, P), Vec<S>)>,
        P: Into<String>,
        S: Into<String>,
    {
        let rules = entries
            .into_iter()
            .map(|((method, path), perms)| {
                (
                    (method, path.into()),
                    perms.into_iter().map(|p| p.into()).collect(),
                )
            })
            .collect();
        Self { rules }
    }

    /// Look up the required permissions for a given method and path.
    ///
    /// Returns `None` when the route is not configured (unprotected).
    pub fn required_perms(&self, method: &Method, path: &str) -> Option<&Vec<String>> {
        self.rules.get(&(method.clone(), path.to_string()))
    }
}

/// Axum middleware function that enforces route-level permission checks.
///
/// Extracted from request extensions by key type `VerifiedClaims` (set by the
/// auth middleware that runs before this one in the stack).
///
/// Decision table:
///
/// | Route in config | Claims present | Has required perm | Result |
/// |-----------------|---------------|-------------------|--------|
/// | No              | —             | —                 | Pass   |
/// | Yes             | No            | —                 | 403    |
/// | Yes             | Yes           | `admin:all`       | Pass   |
/// | Yes             | Yes           | At least one      | Pass   |
/// | Yes             | Yes           | None              | 403    |
pub async fn authz_gate_middleware(
    State(config): State<Arc<AuthzGateConfig>>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let required = match config.required_perms(&method, &path) {
        None => return next.run(req).await,
        Some(perms) => perms.clone(),
    };

    match req.extensions().get::<VerifiedClaims>().cloned() {
        None => {
            tracing::warn!(
                method = %method,
                path = %path,
                "authz denied — no verified claims on protected route",
            );
            platform_http_contracts::ApiError::forbidden("Insufficient permissions").into_response()
        }
        Some(claims) => {
            if claims.perms.iter().any(|p| p == "admin:all") {
                return next.run(req).await;
            }

            if required.iter().any(|r| claims.perms.contains(r)) {
                next.run(req).await
            } else {
                tracing::warn!(
                    method = %method,
                    path = %path,
                    user_id = %claims.user_id,
                    required = ?required,
                    "authz denied — insufficient permissions",
                );
                platform_http_contracts::ApiError::forbidden("Insufficient permissions")
                    .into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::routing::{delete, get, post};
    use axum::Router;
    use chrono::Utc;
    use security::claims::{ActorType, VerifiedClaims};
    use tower::ServiceExt;
    use uuid::Uuid;

    fn make_claims(perms: Vec<&str>) -> VerifiedClaims {
        VerifiedClaims {
            user_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            app_id: None,
            roles: vec![],
            perms: perms.into_iter().map(|s| s.to_string()).collect(),
            actor_type: ActorType::User,
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            token_id: Uuid::new_v4(),
            version: "1".to_string(),
        }
    }

    fn make_app(config: AuthzGateConfig) -> Router {
        let config = Arc::new(config);
        Router::new()
            .route("/api/items", get(|| async { "ok" }))
            .route("/api/items", post(|| async { "ok" }))
            .route("/api/items", delete(|| async { "ok" }))
            .route("/public", get(|| async { "public" }))
            .layer(axum::middleware::from_fn_with_state(
                config,
                authz_gate_middleware,
            ))
    }

    fn req_with_claims(method: Method, uri: &str, claims: VerifiedClaims) -> Request<Body> {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("test assertion");
        req.extensions_mut().insert(claims);
        req
    }

    fn req_no_claims(method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("test assertion")
    }

    fn config() -> AuthzGateConfig {
        AuthzGateConfig::new([
            ((Method::GET, "/api/items"), vec!["items:read"]),
            ((Method::POST, "/api/items"), vec!["items:write"]),
            (
                (Method::DELETE, "/api/items"),
                vec!["items:delete", "items:admin"],
            ),
        ])
    }

    #[tokio::test]
    async fn allowed_request_passes() {
        let app = make_app(config());
        let claims = make_claims(vec!["items:read"]);
        let req = req_with_claims(Method::GET, "/api/items", claims);
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn denied_request_returns_403() {
        let app = make_app(config());
        let claims = make_claims(vec!["items:read"]); // no write perm
        let req = req_with_claims(Method::POST, "/api/items", claims);
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_all_bypasses_check() {
        let app = make_app(config());
        let claims = make_claims(vec!["admin:all"]);
        // POST requires items:write, but admin:all overrides
        let req = req_with_claims(Method::POST, "/api/items", claims);
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unprotected_route_passes_without_claims() {
        let app = make_app(config());
        let req = req_no_claims(Method::GET, "/public");
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn protected_route_without_claims_returns_403() {
        let app = make_app(config());
        let req = req_no_claims(Method::GET, "/api/items");
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn any_of_permissions_allows_access() {
        let app = make_app(config());
        // DELETE requires "items:delete" OR "items:admin" — user has items:admin
        let claims = make_claims(vec!["items:admin"]);
        let req = req_with_claims(Method::DELETE, "/api/items", claims);
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn wrong_perm_on_multi_perm_route_denied() {
        let app = make_app(config());
        // DELETE requires "items:delete" OR "items:admin" — user has only items:read
        let claims = make_claims(vec!["items:read"]);
        let req = req_with_claims(Method::DELETE, "/api/items", claims);
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn empty_perms_config_produces_empty_guard() {
        let config = AuthzGateConfig::new::<Vec<((Method, &str), Vec<&str>)>, &str, &str>(vec![]);
        let app = make_app(config);
        // Everything passes — no rules configured
        let req = req_no_claims(Method::GET, "/api/items");
        let resp = app.oneshot(req).await.expect("test assertion");
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

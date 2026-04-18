use std::sync::Arc;
use std::time::Duration;

use security::claims::JwtVerifier;

use crate::config::WebAuthConfig;
use crate::handlers::build_router;
use crate::middleware::CookieAuthLayer;

/// Entry point for constructing a WebAuthProxy.
pub struct WebAuthProxy;

impl WebAuthProxy {
    pub fn builder() -> WebAuthProxyBuilder {
        WebAuthProxyBuilder::default()
    }
}

/// Builder for the BFF auth proxy.
///
/// Call `.build()` to receive an `(axum::Router, CookieAuthLayer)` pair.
/// Mount the router under `/api/auth` and apply the layer globally:
///
/// ```rust,ignore
/// let (router, cookie_mw) = WebAuthProxy::builder()
///     .cookie_prefix("huber")
///     .identity_auth_url(auth_url)
///     .build()?;
/// app.nest("/api/auth", router).layer(cookie_mw);
/// ```
pub struct WebAuthProxyBuilder {
    cookie_prefix: String,
    refresh_cookie_path: String,
    identity_auth_url: String,
    access_cookie_max_age: Duration,
    refresh_cookie_max_age: Duration,
    force_secure: Option<bool>,
    verifier: Option<Arc<JwtVerifier>>,
}

impl Default for WebAuthProxyBuilder {
    fn default() -> Self {
        Self {
            cookie_prefix: "7d".to_string(),
            refresh_cookie_path: "/api/auth".to_string(),
            identity_auth_url: String::new(),
            access_cookie_max_age: Duration::from_secs(60 * 30),
            refresh_cookie_max_age: Duration::from_secs(60 * 60 * 24 * 30),
            force_secure: None,
            verifier: None,
        }
    }
}

impl WebAuthProxyBuilder {
    /// Cookie name prefix. Access cookie: `{prefix}_session`. Refresh: `{prefix}_refresh`.
    pub fn cookie_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.cookie_prefix = prefix.into();
        self
    }

    /// `Path` attribute for the refresh cookie (narrows browser sends to auth routes only).
    pub fn refresh_cookie_path(mut self, path: impl Into<String>) -> Self {
        self.refresh_cookie_path = path.into();
        self
    }

    /// Base URL of the identity-auth service (e.g. `http://identity-auth:3000`).
    pub fn identity_auth_url(mut self, url: impl Into<String>) -> Self {
        self.identity_auth_url = url.into();
        self
    }

    /// `Max-Age` for the access cookie. Defaults to 30 minutes.
    pub fn access_cookie_max_age(mut self, age: Duration) -> Self {
        self.access_cookie_max_age = age;
        self
    }

    /// `Max-Age` for the refresh cookie. Defaults to 30 days.
    ///
    /// Set this to match `REFRESH_ABSOLUTE_MAX_DAYS` on the server — the cookie
    /// should outlast the idle window so the server remains authoritative for
    /// idle expiry, not the cookie.
    pub fn refresh_cookie_max_age(mut self, age: Duration) -> Self {
        self.refresh_cookie_max_age = age;
        self
    }

    /// Override the `Secure` cookie attribute regardless of `APP_ENV`.
    /// When not set, `Secure=true` when `APP_ENV=production` (case-insensitive).
    pub fn force_secure(mut self, secure: bool) -> Self {
        self.force_secure = Some(secure);
        self
    }

    /// Provide an explicit JWT verifier instead of loading from the environment.
    /// Useful in tests or when the caller manages the verifier lifecycle.
    pub fn with_verifier(mut self, verifier: Arc<JwtVerifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    /// Build the router and cookie middleware layer.
    ///
    /// Reads `JWT_PUBLIC_KEY` (or `JWT_PUBLIC_KEY_PEM` / `JWT_PUBLIC_KEY_PREV`)
    /// from the environment for JWT verification. If not set, the `/me` endpoint
    /// and the cookie middleware will pass through without claims.
    pub fn build(self) -> Result<(axum::Router, CookieAuthLayer), BuildError> {
        if self.identity_auth_url.is_empty() {
            return Err(BuildError::MissingAuthUrl);
        }

        let secure = self.force_secure.unwrap_or_else(|| {
            std::env::var("APP_ENV")
                .map(|v| v.eq_ignore_ascii_case("production"))
                .unwrap_or(false)
        });

        let jwt_verifier = self
            .verifier
            .or_else(|| JwtVerifier::from_env_with_overlap().map(Arc::new));

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(BuildError::HttpClient)?;

        let config = Arc::new(WebAuthConfig {
            cookie_prefix: self.cookie_prefix,
            refresh_cookie_path: self.refresh_cookie_path,
            access_max_age_secs: self.access_cookie_max_age.as_secs() as i64,
            refresh_max_age_secs: self.refresh_cookie_max_age.as_secs() as i64,
            secure,
            auth_base_url: self.identity_auth_url,
            http_client,
            jwt_verifier: jwt_verifier.clone(),
        });

        let router = build_router(config.clone());
        let layer = CookieAuthLayer::new(config);

        Ok((router, layer))
    }
}

/// Errors returned by [`WebAuthProxyBuilder::build`].
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("identity_auth_url is required — call .identity_auth_url(url) before .build()")]
    MissingAuthUrl,
    #[error("failed to build HTTP client: {0}")]
    HttpClient(reqwest::Error),
}

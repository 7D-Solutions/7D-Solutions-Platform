//! Configuration for the auth kit.

use std::time::Duration;

/// Configuration for connecting to the identity-auth service.
#[derive(Debug, Clone)]
pub struct AuthKitConfig {
    /// Base URL of the identity-auth service (e.g. "http://identity-auth:3000").
    pub identity_url: String,

    /// JWKS endpoint URL. Defaults to `{identity_url}/api/auth/jwks`.
    pub jwks_url: Option<String>,

    /// How often to refresh the JWKS key set (default: 5 minutes).
    pub jwks_refresh_interval: Duration,

    /// Fall back to `JWT_PUBLIC_KEY` env var if JWKS fetch fails.
    pub fallback_to_env: bool,
}

impl AuthKitConfig {
    /// Create a config pointing at the given identity-auth base URL.
    pub fn new(identity_url: impl Into<String>) -> Self {
        Self {
            identity_url: identity_url.into(),
            jwks_url: None,
            jwks_refresh_interval: Duration::from_secs(300),
            fallback_to_env: true,
        }
    }

    /// Override the JWKS endpoint URL.
    pub fn with_jwks_url(mut self, url: impl Into<String>) -> Self {
        self.jwks_url = Some(url.into());
        self
    }

    /// Set the JWKS refresh interval.
    pub fn with_refresh_interval(mut self, interval: Duration) -> Self {
        self.jwks_refresh_interval = interval;
        self
    }

    /// Resolved JWKS URL: explicit override or default derived from identity_url.
    pub(crate) fn resolved_jwks_url(&self) -> String {
        self.jwks_url
            .clone()
            .unwrap_or_else(|| format!("{}/api/auth/jwks", self.identity_url))
    }
}

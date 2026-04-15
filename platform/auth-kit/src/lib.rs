//! Vertical app auth kit — drop-in authentication for 7D platform verticals.
//!
//! Provides login/logout/refresh proxy handlers, JWKS-backed JWT verification,
//! claims extraction middleware, and permission checking. Verticals add this
//! crate as a dependency and write zero auth code.
//!
//! # Usage
//!
//! ```rust,ignore
//! use auth_kit::{AuthKit, AuthKitConfig};
//! use axum::Router;
//!
//! let config = AuthKitConfig::new("http://identity-auth:3000");
//! let auth = AuthKit::init(config).await?;
//!
//! let app = Router::new()
//!     // Vertical's own routes — protected by JWT claims middleware
//!     .route("/api/orders", get(list_orders))
//!     .route_layer(auth.require_permissions(&["orders.read"]))
//!     .merge(auth.proxy_routes())          // /auth/login, /auth/refresh, /auth/logout
//!     .layer(auth.claims_layer());         // JWT verification on all routes
//! ```

pub mod config;
pub mod proxy;

pub use config::AuthKitConfig;

// Re-export security primitives so verticals don't need a direct dependency.
pub use security::authz_middleware::{ClaimsLayer, RequirePermissionsLayer};
pub use security::claims::{ActorType, JwtVerifier, VerifiedClaims};
pub use security::{check_permissions, SecurityError};

use axum::routing::post;
use axum::Router;
use std::sync::Arc;
use std::time::Duration;

/// Internal shared state for proxy handlers.
pub(crate) struct AuthKitState {
    pub(crate) identity_url: String,
    pub(crate) http: reqwest::Client,
}

/// Initialized auth kit — provides middleware layers and proxy routes.
pub struct AuthKit {
    verifier: Arc<JwtVerifier>,
    state: Arc<AuthKitState>,
}

/// Errors during auth kit initialization.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("JWKS initialization failed: {0}")]
    Jwks(#[from] SecurityError),
}

impl AuthKit {
    /// Initialize the auth kit: fetch JWKS keys and start background refresh.
    pub async fn init(config: AuthKitConfig) -> Result<Self, InitError> {
        let jwks_url = config.resolved_jwks_url();
        let verifier = JwtVerifier::from_jwks_url(
            &jwks_url,
            config.jwks_refresh_interval,
            config.fallback_to_env,
        )
        .await?;

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");

        Ok(Self {
            verifier: Arc::new(verifier),
            state: Arc::new(AuthKitState {
                identity_url: config.identity_url,
                http,
            }),
        })
    }

    /// Create from existing components (useful for testing or custom setups).
    pub fn from_parts(verifier: Arc<JwtVerifier>, identity_url: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");

        Self {
            verifier,
            state: Arc::new(AuthKitState { identity_url, http }),
        }
    }

    /// Claims extraction layer — verifies Bearer tokens and inserts
    /// `VerifiedClaims` into request extensions. Permissive mode: missing
    /// tokens pass through (use `require_permissions` on mutation routes).
    pub fn claims_layer(&self) -> ClaimsLayer {
        ClaimsLayer::permissive(self.verifier.clone())
    }

    /// Strict claims layer — returns 401 for any request without a valid JWT.
    pub fn strict_claims_layer(&self) -> ClaimsLayer {
        ClaimsLayer::strict(self.verifier.clone())
    }

    /// Permission guard layer — returns 403 if the caller lacks any of
    /// the listed permissions. Stack on mutation routes.
    pub fn require_permissions(&self, perms: &[&str]) -> RequirePermissionsLayer {
        RequirePermissionsLayer::new(perms)
    }

    /// Proxy routes for login, refresh, and logout.
    ///
    /// Returns a router with:
    /// - POST /auth/login
    /// - POST /auth/refresh
    /// - POST /auth/logout
    pub fn proxy_routes(&self) -> Router {
        Router::new()
            .route("/auth/login", post(proxy::login))
            .route("/auth/refresh", post(proxy::refresh))
            .route("/auth/logout", post(proxy::logout))
            .with_state(self.state.clone())
    }

    /// Access the underlying JWT verifier for direct token validation.
    pub fn verifier(&self) -> &Arc<JwtVerifier> {
        &self.verifier
    }
}

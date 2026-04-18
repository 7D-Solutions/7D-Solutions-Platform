//! BFF cookie auth proxy for 7D platform vertical backends.
//!
//! Provides a drop-in `WebAuthProxy` builder that returns:
//! - An Axum `Router` with POST /login, POST /logout, GET /me, POST /refresh
//! - A `CookieAuthLayer` that reads the `{prefix}_session` access cookie,
//!   validates the JWT, and inserts `Extension<VerifiedClaims>` on success
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_web_auth::WebAuthProxy;
//! use std::time::Duration;
//!
//! let (auth_router, cookie_mw) = WebAuthProxy::builder()
//!     .cookie_prefix("huber")
//!     .refresh_cookie_path("/api/auth")
//!     .identity_auth_url("http://identity-auth:3000")
//!     .access_cookie_max_age(Duration::from_secs(60 * 30))
//!     .refresh_cookie_max_age(Duration::from_secs(60 * 60 * 24 * 30))
//!     .build()?;
//!
//! let app = axum::Router::new()
//!     .route("/api/things", get(list_things))
//!     .nest("/api/auth", auth_router)
//!     .layer(cookie_mw);
//! ```
//!
//! # Cookie naming
//!
//! | Cookie | Name | Path | Notes |
//! |--------|------|------|-------|
//! | Access token | `{prefix}_session` | `/` | HttpOnly, SameSite=Lax |
//! | Refresh token | `{prefix}_refresh` | `refresh_cookie_path` | HttpOnly, SameSite=Lax |
//!
//! # Secure attribute
//!
//! Set automatically when `APP_ENV=production` (case-insensitive).
//! Override via `.force_secure(bool)`.

mod builder;
mod config;
mod handlers;
mod middleware;

pub use builder::{BuildError, WebAuthProxy, WebAuthProxyBuilder};
pub use middleware::{CookieAuthLayer, CookieAuthMiddleware};

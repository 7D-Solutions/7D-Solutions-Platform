pub mod auth;
pub mod config;
pub mod http;
pub mod metrics;
pub mod outbox;

use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use config::Config;
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: metrics::PortalMetrics,
    pub portal_jwt: Arc<auth::PortalJwt>,
    pub config: Config,
}

pub fn hash_password(password: &str) -> Result<String, password_hash::Error> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .ok()
        .and_then(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).ok())
        .is_some()
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let admin_routes = Router::new()
        .route("/portal/admin/users", axum::routing::post(http::admin::invite_user))
        .route_layer(RequirePermissionsLayer::new(&[permissions::PARTY_MUTATE]))
        .with_state(state.clone());

    let auth_routes = Router::new()
        .route("/portal/auth/login", axum::routing::post(http::auth::login))
        .route("/portal/auth/refresh", axum::routing::post(http::auth::refresh))
        .route("/portal/auth/logout", axum::routing::post(http::auth::logout))
        .route("/portal/me", get(http::protected::me))
        .route(
            "/portal/party/{party_id}/probe",
            get(http::protected::party_guard_probe),
        )
        .with_state(state.clone());

    Router::new()
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .route("/metrics", get(metrics::metrics_handler))
        .merge(admin_routes)
        .merge(auth_routes)
        .with_state(state.clone())
        .layer(Extension(state.portal_jwt.clone()))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            optional_claims_mw,
        ))
        .layer(build_cors_layer(&state.config))
}

fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

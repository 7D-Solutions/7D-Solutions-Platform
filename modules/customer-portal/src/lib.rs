pub mod auth;
pub mod config;
pub mod http;
pub mod metrics;
pub mod outbox;

use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{extract::DefaultBodyLimit, routing::get, Extension, Json, Router};
use config::Config;
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use utoipa::OpenApi;

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
        .and_then(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .ok()
        })
        .is_some()
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let admin_routes = Router::new()
        .route(
            "/portal/admin/users",
            axum::routing::post(http::admin::invite_user),
        )
        .route(
            "/portal/admin/docs/link",
            axum::routing::post(http::status::link_document),
        )
        .route(
            "/portal/admin/status-cards",
            axum::routing::post(http::status::create_status_card),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::CUSTOMER_PORTAL_ADMIN,
        ]))
        .with_state(state.clone());

    let auth_routes = Router::new()
        .route("/portal/auth/login", axum::routing::post(http::auth::login))
        .route(
            "/portal/auth/refresh",
            axum::routing::post(http::auth::refresh),
        )
        .route(
            "/portal/auth/logout",
            axum::routing::post(http::auth::logout),
        )
        .route("/portal/me", get(http::protected::me))
        .route("/portal/docs", get(http::docs::list_documents))
        .route("/portal/status/feed", get(http::status::list_status_cards))
        .route(
            "/portal/acknowledgments",
            axum::routing::post(http::status::acknowledge),
        )
        .route(
            "/portal/party/{party_id}/probe",
            get(http::protected::party_guard_probe),
        )
        .with_state(state.clone());

    Router::new()
        .route("/api/openapi.json", get(openapi_json))
        .merge(admin_routes)
        .merge(auth_routes)
        .with_state(state.clone())
        .layer(Extension(state.portal_jwt.clone()))
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

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

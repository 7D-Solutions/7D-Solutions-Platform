use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use workforce_competence_rs::{
    db::resolver::resolve_pool,
    http::{
        handlers::{
            get_acceptance_authority_check, get_artifact, get_authorization, post_artifact,
            post_assignment, post_grant_authority, post_revoke_authority,
        },
        health::{health, ready, schema_version, version},
    },
    metrics::{metrics_handler, WcMetrics},
    AppState, Config,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting Workforce Competence service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: host={}, port={}",
        config.host,
        config.port
    );

    tracing::info!("Connecting to database...");
    let pool = resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let shutdown_pool = pool.clone();

    let metrics = Arc::new(WcMetrics::new().expect("Failed to create metrics registry"));
    let app_state = Arc::new(AppState { pool, metrics });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let wc_mutations = Router::new()
        .route(
            "/api/workforce-competence/artifacts",
            axum::routing::post(post_artifact),
        )
        .route(
            "/api/workforce-competence/assignments",
            axum::routing::post(post_assignment),
        )
        .route(
            "/api/workforce-competence/acceptance-authorities",
            axum::routing::post(post_grant_authority),
        )
        .route(
            "/api/workforce-competence/acceptance-authorities/{id}/revoke",
            axum::routing::post(post_revoke_authority),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::WORKFORCE_COMPETENCE_MUTATE,
        ]))
        .with_state(app_state.clone());

    let wc_reads = Router::new()
        .route(
            "/api/workforce-competence/artifacts/{id}",
            axum::routing::get(get_artifact),
        )
        .route(
            "/api/workforce-competence/authorization",
            axum::routing::get(get_authorization),
        )
        .route(
            "/api/workforce-competence/acceptance-authority-check",
            axum::routing::get(get_acceptance_authority_check),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::WORKFORCE_COMPETENCE_READ,
        ]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/api/schema-version", get(schema_version))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .merge(wc_reads)
        .merge(wc_mutations)
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
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Workforce Competence service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received — draining in-flight requests");
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

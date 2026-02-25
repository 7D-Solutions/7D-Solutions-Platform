use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use axum::Extension;
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, JwtVerifier};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use ttp_rs::{config::Config, db, http, metrics, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("TTP service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "TTP: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("TTP: failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("TTP: failed to run database migrations");

    tracing::info!("TTP: database migrations applied");

    let ttp_metrics = Arc::new(
        metrics::TtpMetrics::new().expect("TTP: failed to create metrics"),
    );
    tracing::info!("TTP: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: ttp_metrics,
    });

    // Optional JWT verifier for claims extraction (requires JWT_PUBLIC_KEY env var).
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = http::router(app_state)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("TTP service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("TTP: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("TTP: failed to start server");
}

fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    if is_wildcard && config.env != "development" {
        tracing::warn!("CORS_ORIGINS is set to wildcard — restrict to specific origins in production");
    }

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
        .allow_credentials(false)
}

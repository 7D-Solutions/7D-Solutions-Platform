use axum::{extract::DefaultBodyLimit, http::Method, Extension};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, JwtVerifier,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use timekeeping::{config::Config, db, http, metrics, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Timekeeping service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Timekeeping: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Timekeeping: failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Timekeeping: failed to run database migrations");

    tracing::info!("Timekeeping: database migrations applied");

    let tk_metrics = Arc::new(
        metrics::TimekeepingMetrics::new().expect("Timekeeping: failed to create metrics"),
    );
    tracing::info!("Timekeeping: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: tk_metrics,
    });

    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse().unwrap(),
            "http://localhost:3000".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ])
        .allow_credentials(true);

    let maybe_verifier = JwtVerifier::from_env().map(Arc::new);

    let app = http::router(app_state)
        .merge(http::admin::admin_router(pool))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(security::AuthzLayer::from_env())
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Timekeeping service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Timekeeping: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Timekeeping: failed to start server");
}

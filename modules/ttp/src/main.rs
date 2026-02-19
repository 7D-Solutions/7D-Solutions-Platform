use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use axum::Extension;
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
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

    let app = http::router(app_state)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(security::AuthzLayer::from_env())
        .layer(cors)
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

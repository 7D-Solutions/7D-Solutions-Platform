use axum::http::Method;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use consolidation::{config::Config, db, http, metrics, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Consolidation service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Consolidation: config loaded: host={}, port={}",
        config.host,
        config.port
    );

    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Consolidation: failed to connect to Postgres");

    tracing::info!("Consolidation: database connected");

    let consolidation_metrics = Arc::new(
        metrics::ConsolidationMetrics::new()
            .expect("Consolidation: failed to create metrics"),
    );
    tracing::info!("Consolidation: metrics initialized");

    let app_state = Arc::new(AppState {
        pool,
        metrics: consolidation_metrics,
        gl_base_url: config.gl_base_url,
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

    let app = http::router()
        .with_state(app_state)
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Consolidation service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Consolidation: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Consolidation: failed to start server");
}

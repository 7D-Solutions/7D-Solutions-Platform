use axum::{routing::get, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use inventory_rs::{
    db::resolver::resolve_pool,
    metrics::{metrics_handler, InventoryMetrics},
    routes::health::{health, ready, version},
    AppState, Config,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting Inventory service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Inventory service cannot start without valid configuration.");
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

    let metrics = Arc::new(
        InventoryMetrics::new().expect("Failed to create metrics registry"),
    );

    let app_state = Arc::new(AppState {
        pool,
        metrics,
    });

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Inventory service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

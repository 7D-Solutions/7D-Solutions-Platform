use axum::{routing::get, Router};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use gl_rs::{config::Config, health::health};

#[tokio::main]
async fn main() {
    // Load environment variables from .env file (if present)
    dotenvy::dotenv().ok();

    // Initialize tracing/logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting GL service...");

    // Load configuration from environment
    let config = Config::from_env()
        .expect("Failed to load configuration from environment");

    tracing::info!(
        "Configuration loaded: host={}, port={}, bus_type={}",
        config.host,
        config.port,
        config.bus_type
    );

    // Build the application router
    let app = Router::new()
        .route("/api/health", get(health))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    // Bind to the configured address
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("GL service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    // Start the server
    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

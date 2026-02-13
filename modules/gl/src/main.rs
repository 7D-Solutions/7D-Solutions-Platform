use axum::{routing::get, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use gl_rs::{
    config::Config,
    health::health,
    start_gl_posting_consumer,
    start_gl_reversal_consumer,
};

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

    // Database connection
    tracing::info!("Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    tracing::info!("Running migrations...");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create event bus
    let bus: Arc<dyn EventBus> = match config.bus_type.to_lowercase().as_str() {
        "inmemory" => {
            tracing::info!("Using InMemory event bus");
            Arc::new(InMemoryBus::new())
        }
        "nats" => {
            tracing::info!("Connecting to NATS at {}", config.nats_url);
            let client = async_nats::connect(&config.nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        _ => panic!("Invalid BUS_TYPE: {}. Must be 'inmemory' or 'nats'", config.bus_type),
    };

    // Start GL posting consumer
    let consumer_pool = pool.clone();
    let consumer_bus = bus.clone();
    start_gl_posting_consumer(consumer_bus, consumer_pool).await;

    // Start GL reversal consumer
    let reversal_pool = pool.clone();
    let reversal_bus = bus.clone();
    start_gl_reversal_consumer(reversal_bus, reversal_pool).await;

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

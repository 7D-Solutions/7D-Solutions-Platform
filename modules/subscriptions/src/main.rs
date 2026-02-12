mod config;
mod consumer;
mod envelope;
mod envelope_validation;
mod models;
mod outbox;
mod publisher;
mod routes;

use axum::{extract::State, routing::get, Json, Router};
use config::Config;
use event_bus::{EventBus, InMemoryBus, NatsBus};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Load configuration
    let config = Config::from_env().expect("Failed to load configuration");
    tracing::info!("Configuration loaded: {:?}", config.bus_type);

    // Initialize database pool
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("Database connection established");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    tracing::info!("Database migrations completed");

    // Initialize event bus
    let bus: Arc<dyn EventBus> = match config.bus_type {
        config::BusType::Nats => {
            let nats_url = config.nats_url.as_ref().expect("NATS_URL required for NATS bus");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = async_nats::connect(nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        config::BusType::InMemory => {
            tracing::info!("Using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn background publisher task
    let publisher_pool = pool.clone();
    let publisher_bus = bus.clone();
    tokio::spawn(async move {
        publisher::run_publisher(publisher_pool, publisher_bus).await;
    });

    tracing::info!("Background event publisher started");

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8087".to_string())
        .parse()
        .expect("PORT must be a valid u16");

    let app = Router::new()
        .route("/api/health", get(health))
        .with_state(pool.clone())
        .merge(routes::subscriptions_router(pool.clone()))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Subscriptions module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

async fn health(State(_pool): State<sqlx::PgPool>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "module": "subscriptions",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

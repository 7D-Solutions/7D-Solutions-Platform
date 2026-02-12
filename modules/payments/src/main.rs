use axum::{routing::get, Json, Router};
use event_bus::{EventBus, InMemoryBus};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

mod events;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Configuration
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    let bus_type = std::env::var("BUS_TYPE")
        .unwrap_or_else(|_| "inmemory".to_string());
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8088".to_string())
        .parse()
        .expect("PORT must be a valid u16");

    // Database connection
    tracing::info!("Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    tracing::info!("Running migrations...");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create event bus
    let bus: Arc<dyn EventBus> = match bus_type.to_lowercase().as_str() {
        "inmemory" => {
            tracing::info!("Using InMemory event bus");
            Arc::new(InMemoryBus::new())
        }
        "nats" => {
            let nats_url = std::env::var("NATS_URL")
                .unwrap_or_else(|_| "nats://localhost:4222".to_string());
            tracing::info!("Connecting to NATS at {}", nats_url);
            let client = async_nats::connect(&nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(event_bus::NatsBus::new(client))
        }
        _ => panic!("Invalid BUS_TYPE: {}. Must be 'inmemory' or 'nats'", bus_type),
    };

    // Spawn outbox publisher task
    let publisher_pool = pool.clone();
    let publisher_bus = bus.clone();
    tokio::spawn(async move {
        tracing::info!("Starting outbox publisher...");
        if let Err(e) = events::outbox::start_outbox_publisher(publisher_pool, publisher_bus).await {
            tracing::error!("Outbox publisher error: {}", e);
        }
    });

    // Spawn payment collection consumer task
    let consumer_pool = pool.clone();
    let consumer_bus = bus.clone();
    payments_rs::start_payment_collection_consumer(consumer_bus, consumer_pool).await;

    let app = Router::new()
        .route("/api/health", get(health))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Payments module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "module": "payments",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

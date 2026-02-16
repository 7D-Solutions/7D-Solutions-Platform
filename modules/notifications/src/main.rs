mod db;
mod consumer;
mod consumer_tasks;
mod dlq;
mod envelope_validation;
mod event_bus;
mod handlers;
mod models;
mod routes;

use axum::{routing::get, Json, Router};
use ::event_bus::{EventBus, InMemoryBus, NatsBus};
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

    // Database configuration
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let db = db::resolver::resolve_pool(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&db)
        .await
        .expect("Failed to run database migrations");

    tracing::info!("Database migrations applied successfully");

    // Event bus configuration
    let bus_type = std::env::var("BUS_TYPE").unwrap_or_else(|_| "inmemory".to_string());
    let bus: Arc<dyn EventBus> = match bus_type.as_str() {
        "nats" => {
            let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = async_nats::connect(&nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        "inmemory" => {
            tracing::info!("Using InMemoryBus for event messaging");
            Arc::new(InMemoryBus::new())
        }
        _ => panic!("Invalid BUS_TYPE: {}. Must be 'nats' or 'inmemory'", bus_type),
    };

    // Spawn outbox publisher task
    tokio::spawn(event_bus::start_outbox_publisher(db.clone(), bus.clone()));

    // Spawn event consumer tasks
    consumer_tasks::start_invoice_issued_consumer(bus.clone(), db.clone()).await;
    consumer_tasks::start_payment_succeeded_consumer(bus.clone(), db.clone()).await;
    consumer_tasks::start_payment_failed_consumer(bus.clone(), db.clone()).await;

    // HTTP server configuration
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8089".to_string())
        .parse()
        .expect("PORT must be a valid u16");

    let app = Router::new()
        .route("/api/health", get(routes::health::health))
        .route("/api/ready", get(routes::health::ready))
        .with_state(db)
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Notifications module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

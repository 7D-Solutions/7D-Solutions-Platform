use axum::{routing::get, Json, Router};
use event_bus::{EventBus, InMemoryBus};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

mod config;
mod db;
mod events;
mod metrics;
mod routes;

use config::Config;

/// Handler for /metrics endpoint
async fn metrics_handler() -> String {
    use prometheus_client::encoding::text::encode;

    // Encode prometheus-client metrics
    let registry = metrics::METRICS_REGISTRY.lock().unwrap();
    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();

    // Append standard prometheus metrics (projection metrics)
    use prometheus::{Encoder, TextEncoder};
    let encoder = TextEncoder::new();
    let projection_families = metrics::PROJECTION_METRICS.registry().gather();
    let mut projection_buffer = Vec::new();
    encoder.encode(&projection_families, &mut projection_buffer)
        .expect("Failed to encode projection metrics");

    buffer.push_str("\n");
    buffer.push_str(&String::from_utf8(projection_buffer)
        .expect("Failed to convert projection metrics to UTF-8"));

    buffer
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Load and validate configuration (fail-fast on missing/invalid config)
    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Payments service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Database connection
    tracing::info!("Connecting to database...");
    let pool = db::resolver::resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    tracing::info!("Running migrations...");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create event bus
    let bus: Arc<dyn EventBus> = match config.bus_type {
        config::BusType::InMemory => {
            tracing::info!("Using InMemory event bus");
            Arc::new(InMemoryBus::new())
        }
        config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(event_bus::NatsBus::new(client))
        }
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
        .route("/api/health", get(routes::health::health))
        .route("/api/ready", get(routes::health::ready))
        .route("/api/version", get(routes::health::version))
        .route("/metrics", get(metrics_handler))
        .with_state(pool)
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");
    tracing::info!("Payments module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

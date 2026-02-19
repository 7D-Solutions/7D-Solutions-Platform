use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use event_bus::{EventBus, InMemoryBus};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

mod db;

use payments_rs::Config;

/// Handler for /metrics endpoint
async fn metrics_handler() -> String {
    use prometheus_client::encoding::text::encode;

    // Encode prometheus-client metrics
    let registry = payments_rs::metrics::METRICS_REGISTRY.lock().unwrap();
    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();

    // Append standard prometheus metrics (projection + SLO metrics)
    use prometheus::{Encoder, TextEncoder};
    let encoder = TextEncoder::new();

    let projection_families = payments_rs::metrics::PROJECTION_METRICS.registry().gather();
    let mut prometheus_buffer = Vec::new();
    encoder.encode(&projection_families, &mut prometheus_buffer)
        .expect("Failed to encode projection metrics");

    // SLO metrics (latency, error rate, consumer lag)
    let slo_families = payments_rs::metrics::SLO_REGISTRY.gather();
    encoder.encode(&slo_families, &mut prometheus_buffer)
        .expect("Failed to encode SLO metrics");

    buffer.push_str("\n");
    buffer.push_str(&String::from_utf8(prometheus_buffer)
        .expect("Failed to convert metrics to UTF-8"));

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
        payments_rs::config::BusType::InMemory => {
            tracing::info!("Using InMemory event bus");
            Arc::new(InMemoryBus::new())
        }
        payments_rs::config::BusType::Nats => {
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
        if let Err(e) = payments_rs::events::outbox::start_outbox_publisher(publisher_pool, publisher_bus).await {
            tracing::error!("Outbox publisher error: {}", e);
        }
    });

    // Spawn payment collection consumer task
    let consumer_pool = pool.clone();
    let consumer_bus = bus.clone();
    payments_rs::start_payment_collection_consumer(consumer_bus, consumer_pool).await;

    // Create application state
    let app_state = Arc::new(payments_rs::AppState {
        pool: pool.clone(),
    });

    let app = Router::new()
        .route("/api/health", get(payments_rs::routes::health::health))
        .route("/api/ready", get(payments_rs::routes::health::ready))
        .route("/api/version", get(payments_rs::routes::health::version))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(security::AuthzLayer::from_env())
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .into_make_service_with_connect_info::<SocketAddr>();

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
